//! 1Password references, read with the `op` CLI at use time (D14).
//!
//! `op read --no-newline -- <reference>` runs from a fixed argument list,
//! never a shell, with a short allowlisted environment, stdin closed and
//! stdout capped. Its stderr is only classified: it is never shown, logged
//! or kept in an error.

use bruhs_models::vault::VaultAttention;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt};
use zeroize::Zeroizing;

/// Longest a read may take, so the app integration's prompt has time.
pub const ONE_PASSWORD_WAIT: Duration = Duration::from_secs(25);

/// Most bytes of value `op` may print (64 KiB).
const MAX_OUTPUT: usize = 64 * 1024;
/// Bytes of stderr kept for classifying; the rest is read and dropped.
const MAX_ERROR: usize = 4 * 1024;
/// Set to an absolute path, it names the `op` to run (tests, odd installs).
const PATH_OVERRIDE: &str = "BRUHS_OP_PATH";
/// Where Homebrew puts `op` when a launchd service's `PATH` lacks it.
const FALLBACKS: [&str; 2] = ["/opt/homebrew/bin/op", "/usr/local/bin/op"];
/// The only variables `op` gets, besides every `OP_*` one (ruling F14).
const PASSED_ENV: [&str; 9] = [
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "TMPDIR",
    "LANG",
    "LC_ALL",
    "XDG_CONFIG_HOME",
    "XDG_RUNTIME_DIR",
];

/// Why a read failed. The text is owner copy; it never holds stderr.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OnePasswordError {
    #[error("1Password CLI isn't installed.")]
    Missing,
    #[error("1Password needs you. Sign in to the 1Password app, then check again.")]
    NeedsOwner,
    #[error("1Password has no item at that reference.")]
    NotFound,
    #[error("Bruhs couldn't read this entry. Check it again.")]
    Failed,
}

impl OnePasswordError {
    #[must_use]
    pub fn attention(self) -> VaultAttention {
        match self {
            Self::Missing => VaultAttention::OnePasswordMissing,
            Self::NeedsOwner => VaultAttention::OnePasswordNeedsYou,
            Self::NotFound => VaultAttention::OnePasswordNotFound,
            Self::Failed => VaultAttention::Failed,
        }
    }
}

/// The `op` binary this process reads references with, if any.
#[derive(Clone, Debug, Default)]
pub struct OnePassword {
    program: Option<PathBuf>,
    wait: Duration,
}

impl OnePassword {
    /// Find `op`: an absolute `BRUHS_OP_PATH` wins, then `PATH`, then
    /// Homebrew's two folders. Looks at files only; never runs it.
    #[must_use]
    pub fn system() -> Self {
        let program = std::env::var_os(PATH_OVERRIDE)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| on_path(std::env::var_os("PATH")))
            .or_else(|| {
                FALLBACKS
                    .iter()
                    .map(PathBuf::from)
                    .find(|path| is_program(path))
            });
        Self {
            program,
            wait: ONE_PASSWORD_WAIT,
        }
    }

    /// A given `op` and wait.
    #[must_use]
    pub fn at(program: PathBuf, wait: Duration) -> Self {
        Self {
            program: Some(program),
            wait,
        }
    }

    /// Whether `op` is there to run. Never runs it, so it never prompts.
    #[must_use]
    pub fn installed(&self) -> bool {
        self.program.as_deref().is_some_and(is_program)
    }

    /// Read one `op://` reference. One trailing newline is stripped; an
    /// empty answer, more than 64 KiB or a non-UTF-8 one is `Failed`, and
    /// running past the wait is `NeedsOwner` (the process is killed).
    pub async fn read(&self, reference: &str) -> Result<Zeroizing<String>, OnePasswordError> {
        let Some(program) = self.program.as_deref().filter(|path| is_program(path)) else {
            return Err(OnePasswordError::Missing);
        };
        let mut command = tokio::process::Command::new(program);
        command
            .args(["read", "--no-newline", "--", reference])
            .env_clear()
            .envs(passed_env())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // Its own process group, so a timeout stops anything it started too.
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command.spawn().map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => OnePasswordError::Missing,
            _ => OnePasswordError::Failed,
        })?;
        #[cfg(unix)]
        let _group = super::ProcessGroup(child.id().and_then(|id| i32::try_from(id).ok()));
        let (Some(mut stdout), Some(mut stderr)) = (child.stdout.take(), child.stderr.take())
        else {
            return Err(OnePasswordError::Failed);
        };
        let outcome = tokio::time::timeout(self.wait, async {
            // stderr is drained alongside, so `op` never blocks on it; a
            // stdout past the cap ends the read at once.
            let (output, errors) = tokio::try_join!(read_output(&mut stdout), async {
                Ok(read_errors(&mut stderr).await)
            })?;
            let status = child.wait().await.map_err(|_| OnePasswordError::Failed)?;
            Ok::<_, OnePasswordError>((status.success(), output, errors))
        })
        .await;
        let (success, output, errors) = match outcome {
            Ok(Ok(read)) => read,
            Ok(Err(error)) => {
                let _ = child.start_kill();
                return Err(error);
            }
            Err(_) => {
                let _ = child.start_kill();
                return Err(OnePasswordError::NeedsOwner);
            }
        };
        if success {
            value(output)
        } else {
            Err(classify(&errors, reference))
        }
    }
}

/// `op` on `PATH`, absolute folders only.
fn on_path(path: Option<OsString>) -> Option<PathBuf> {
    std::env::split_paths(&path?)
        .filter(|folder| folder.is_absolute())
        .map(|folder| folder.join("op"))
        .find(|path| is_program(path))
}

/// A file that may be run.
fn is_program(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

/// The allowlisted part of this process's environment.
fn passed_env() -> Vec<(OsString, OsString)> {
    std::env::vars_os()
        .filter(|(name, _)| {
            name.to_str()
                .is_some_and(|name| PASSED_ENV.contains(&name) || name.starts_with("OP_"))
        })
        .collect()
}

/// stdout up to the cap, in one buffer that never reallocates.
async fn read_output(
    reader: &mut (impl AsyncRead + Unpin),
) -> Result<Zeroizing<Vec<u8>>, OnePasswordError> {
    let mut buffer = Zeroizing::new(vec![0u8; MAX_OUTPUT + 1]);
    let mut length = 0;
    loop {
        let read = reader
            .read(&mut buffer[length..])
            .await
            .map_err(|_| OnePasswordError::Failed)?;
        if read == 0 {
            break;
        }
        length += read;
        if length > MAX_OUTPUT {
            return Err(OnePasswordError::Failed);
        }
    }
    buffer.truncate(length);
    Ok(buffer)
}

/// The start of stderr; the rest is read and dropped.
async fn read_errors(reader: &mut (impl AsyncRead + Unpin)) -> Zeroizing<Vec<u8>> {
    let mut kept = Zeroizing::new(vec![0u8; MAX_ERROR]);
    let mut scratch = Zeroizing::new(vec![0u8; 1024]);
    let mut length = 0;
    loop {
        let read = if length < MAX_ERROR {
            reader.read(&mut kept[length..]).await
        } else {
            reader.read(&mut scratch[..]).await
        };
        match read {
            Ok(0) | Err(_) => break,
            Ok(read) if length < MAX_ERROR => length += read,
            Ok(_) => {}
        }
    }
    kept.truncate(length);
    kept
}

/// A successful read's value: one trailing newline off, never empty.
fn value(mut output: Zeroizing<Vec<u8>>) -> Result<Zeroizing<String>, OnePasswordError> {
    if output.last() == Some(&b'\n') {
        output.pop();
    }
    if output.is_empty() {
        return Err(OnePasswordError::Failed);
    }
    String::from_utf8(std::mem::take(&mut *output))
        .map(Zeroizing::new)
        .map_err(|error| {
            drop(Zeroizing::new(error.into_bytes()));
            OnePasswordError::Failed
        })
}

/// What a failed read's stderr means, matched case-insensitively. The
/// reference and its vault, item and field names are taken out first, so
/// a name like "Desktop App" in a quoted error can't steer the match.
fn classify(errors: &[u8], reference: &str) -> OnePasswordError {
    let mut text = Zeroizing::new(String::from_utf8_lossy(errors).to_lowercase());
    let reference = reference.to_lowercase();
    let path = reference.strip_prefix("op://").unwrap_or(&reference);
    let path = path.split('?').next().unwrap_or_default();
    for word in std::iter::once(reference.as_str()).chain(path.split('/')) {
        text = without_word(&text, word);
    }
    let has = |needles: &[&str]| needles.iter().any(|needle| text.contains(needle));
    if has(&[
        "not currently signed in",
        "account is not signed in",
        "authorization",
        "desktop app",
    ]) {
        OnePasswordError::NeedsOwner
    } else if has(&["isn't an item", "could not find", "no item"]) {
        OnePasswordError::NotFound
    } else {
        OnePasswordError::Failed
    }
}

/// `text` with each whole-word occurrence of `word` replaced by a space.
fn without_word(text: &str, word: &str) -> Zeroizing<String> {
    let mut out = Zeroizing::new(String::with_capacity(text.len()));
    if word.is_empty() {
        out.push_str(text);
        return out;
    }
    let mut rest = text;
    while let Some(at) = rest.find(word) {
        out.push_str(&rest[..at]);
        let after = rest[at + word.len()..].chars().next();
        let bounded = !out.chars().next_back().is_some_and(char::is_alphanumeric)
            && !after.is_some_and(char::is_alphanumeric);
        out.push_str(if bounded { " " } else { word });
        rest = &rest[at + word.len()..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    const WAIT: Duration = Duration::from_secs(10);

    /// A stand-in `op` running `body` under `sh`.
    fn stand_in(folder: &Path, body: &str) -> PathBuf {
        let path = folder.join(format!("op-{}", uuid::Uuid::new_v4().simple()));
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    async fn read_with(body: &str, wait: Duration) -> Result<Zeroizing<String>, OnePasswordError> {
        let folder = tempfile::tempdir().unwrap();
        OnePassword::at(stand_in(folder.path(), body), wait)
            .read("op://P/I/password")
            .await
    }

    #[tokio::test]
    async fn op_reads_a_reference_and_strips_one_newline() {
        let folder = tempfile::tempdir().unwrap();
        let names = folder.path().join("env-names");
        let body = format!(
            r#"/usr/bin/env | /usr/bin/cut -d= -f1 > '{names}'
[ -n "$BRUHS_SECRET_BROKER" ] && exit 9
[ "$#" = 4 ] && [ "$1" = read ] && [ "$2" = --no-newline ] && [ "$3" = -- ] && [ "$4" = op://P/I/password ] || exit 3
printf 'op-canary\n'"#,
            names = names.display()
        );
        let op = OnePassword::at(stand_in(folder.path(), &body), WAIT);
        assert!(op.installed());
        let value = op.read("op://P/I/password").await.unwrap();
        assert_eq!(value.as_str(), "op-canary");
        let names = std::fs::read_to_string(names).unwrap();
        // `sh` itself exports these to the commands it runs.
        let shell = ["PWD", "SHLVL", "_", "OLDPWD"];
        for name in names.lines().filter(|name| !name.is_empty()) {
            assert!(
                PASSED_ENV.contains(&name) || name.starts_with("OP_") || shell.contains(&name),
                "op got {name}"
            );
        }
    }

    #[tokio::test]
    async fn op_failures_map_to_owner_copy_without_stderr() {
        let canary = "op-stderr-canary";
        let cases = [
            (
                "echo '[ERROR] account is not signed in' >&2; exit 1",
                OnePasswordError::NeedsOwner,
            ),
            (
                "echo '[ERROR] \"I\" isn'\"'\"'t an item in any vault.' >&2; exit 1",
                OnePasswordError::NotFound,
            ),
            (
                "echo '[ERROR] Authorization prompt dismissed' >&2; exit 1",
                OnePasswordError::NeedsOwner,
            ),
            (
                &format!("echo 'boom {canary}' >&2; exit 1"),
                OnePasswordError::Failed,
            ),
            ("exit 0", OnePasswordError::Failed),
            ("printf '\\n'", OnePasswordError::Failed),
            (
                "/usr/bin/head -c 71680 /dev/zero | /usr/bin/tr '\\0' a",
                OnePasswordError::Failed,
            ),
        ];
        let mut errors = Vec::new();
        for (body, expected) in cases {
            let error = read_with(body, WAIT).await.unwrap_err();
            assert_eq!(error, expected, "{body}");
            errors.push(error);
        }
        let started = std::time::Instant::now();
        let slow = read_with("/bin/sleep 5; printf late", Duration::from_millis(200)).await;
        assert_eq!(slow.unwrap_err(), OnePasswordError::NeedsOwner);
        assert!(started.elapsed() < Duration::from_secs(3));
        let missing = OnePassword::at("/nonexistent/op".into(), WAIT)
            .read("op://P/I/password")
            .await;
        assert_eq!(missing.unwrap_err(), OnePasswordError::Missing);
        for error in errors {
            assert!(!error.to_string().contains(canary));
            assert!(!format!("{error:?}").contains(canary));
        }
    }

    #[tokio::test]
    async fn a_timed_out_read_kills_op_and_its_children() {
        let folder = tempfile::tempdir().unwrap();
        let started = folder.path().join("started");
        let finished = folder.path().join("finished");
        // A child of `op` that outlives it, as a helper would: the whole
        // group goes.
        let body = format!(
            "( : > '{}'; /bin/sleep 2; : > '{}' ) & wait",
            started.display(),
            finished.display()
        );
        let op = OnePassword::at(stand_in(folder.path(), &body), Duration::from_secs(1));
        assert_eq!(
            op.read("op://P/I/password").await.unwrap_err(),
            OnePasswordError::NeedsOwner
        );
        assert!(started.exists(), "the child ran before the wait ended");
        tokio::time::sleep(Duration::from_millis(2_500)).await;
        assert!(!finished.exists(), "op's child ran on after its wait");
    }

    #[test]
    fn a_missing_cli_is_reported_without_running_it() {
        assert!(!OnePassword::at("/nonexistent/op".into(), WAIT).installed());
        assert!(!OnePassword::default().installed());
        let folder = tempfile::tempdir().unwrap();
        let plain = folder.path().join("op");
        std::fs::write(&plain, "not a program").unwrap();
        assert!(!OnePassword::at(plain, WAIT).installed());
        assert!(on_path(Some(OsString::from("relative/bin"))).is_none());
    }

    #[test]
    fn stderr_is_classified_case_insensitively() {
        let reference = "op://P/I/password";
        assert_eq!(
            classify(b"[ERROR] You are NOT CURRENTLY SIGNED IN.", reference),
            OnePasswordError::NeedsOwner
        );
        assert_eq!(
            classify(b"connecting to Desktop App: timeout", reference),
            OnePasswordError::NeedsOwner
        );
        assert_eq!(
            classify(b"could not find item", reference),
            OnePasswordError::NotFound
        );
        assert_eq!(
            classify(b"No item found", reference),
            OnePasswordError::NotFound
        );
        assert_eq!(classify(b"\xff\xfe", reference), OnePasswordError::Failed);
    }

    #[test]
    fn names_in_the_reference_never_steer_the_match() {
        let reference = "op://Work/Desktop App/Authorization";
        assert_eq!(
            classify(
                b"[ERROR] \"Desktop App\" isn't an item in the \"Work\" vault.",
                reference
            ),
            OnePasswordError::NotFound
        );
        assert_eq!(
            classify(
                b"[ERROR] could not read secret 'op://Work/Desktop App/Authorization': boom",
                reference
            ),
            OnePasswordError::Failed
        );
        // A real sign-in failure still reads as one; only whole words go.
        assert_eq!(
            classify(
                b"[ERROR] account is not signed in",
                "op://Private/Sign/password"
            ),
            OnePasswordError::NeedsOwner
        );
        assert_eq!(without_word("happy app", "app").as_str(), "happy  ");
    }

    #[test]
    fn errors_name_their_attention() {
        assert_eq!(
            OnePasswordError::NeedsOwner.attention(),
            VaultAttention::OnePasswordNeedsYou
        );
        assert_eq!(
            OnePasswordError::NeedsOwner.to_string(),
            VaultAttention::OnePasswordNeedsYou.message()
        );
        assert_eq!(
            OnePasswordError::Missing.attention(),
            VaultAttention::OnePasswordMissing
        );
        assert_eq!(
            OnePasswordError::NotFound.attention(),
            VaultAttention::OnePasswordNotFound
        );
        assert_eq!(OnePasswordError::Failed.attention(), VaultAttention::Failed);
    }
}
