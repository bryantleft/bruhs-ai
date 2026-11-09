//! A Mac app opened from Finder or the Dock gets launchd's PATH
//! (`/usr/bin:/bin:/usr/sbin:/sbin`), not the one the user's shell sets up,
//! so `claude`, `codex`, `git` from Homebrew, `npm` and friends would all be
//! "not installed". At startup the packaged app asks the login shell for its
//! PATH, adds the usual install dirs, and sets
//! the result for itself and every helper and agent run it starts.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MARKER: &str = "__BRUHS_PATH__";
const SHELL_TIMEOUT: Duration = Duration::from_secs(5);

/// Read the login shell's PATH and merge it into this process's PATH.
///
/// Call it first thing in `main`, before anything starts a thread: it sets
/// an environment variable.
pub fn adopt_login_path() {
    let current = std::env::var("PATH").unwrap_or_default();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let login = login_shell_path();
    let merged = merge(login.as_deref(), &current, home.as_deref());
    if merged != current {
        // Runs before the logger, the GPUI app or any runtime exists, so no
        // other thread can be reading the environment.
        std::env::set_var("PATH", merged);
    }
}

fn login_shell_path() -> Option<String> {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| s.starts_with('/'))
        .unwrap_or_else(|| "/bin/zsh".to_string());
    let mut child = Command::new(shell)
        .args(["-ilc", &format!("printf '{MARKER}%s{MARKER}' \"$PATH\"")])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < SHELL_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    between_markers(&out).map(str::to_string)
}

/// The text between the first two markers (shell rc files may print noise
/// around it).
fn between_markers(out: &str) -> Option<&str> {
    let start = out.find(MARKER)? + MARKER.len();
    let len = out[start..].find(MARKER)?;
    Some(&out[start..start + len]).filter(|p| !p.is_empty())
}

/// Login PATH first, then the current PATH, then common install dirs; each
/// entry once, empty and relative entries dropped.
fn merge(login: Option<&str>, current: &str, home: Option<&Path>) -> String {
    let mut extra: Vec<String> = Vec::new();
    if let Some(home) = home {
        for dir in [
            ".local/bin",
            ".bun/bin",
            ".volta/bin",
            ".local/share/mise/shims",
            ".cargo/bin",
        ] {
            extra.push(home.join(dir).to_string_lossy().into_owned());
        }
    }
    extra.extend(["/opt/homebrew/bin", "/opt/homebrew/sbin", "/usr/local/bin"].map(str::to_string));
    let mut seen: Vec<&str> = Vec::new();
    let sources = login
        .into_iter()
        .flat_map(|p| p.split(':'))
        .chain(current.split(':'))
        .chain(extra.iter().map(String::as_str));
    for entry in sources {
        if entry.starts_with('/') && !seen.contains(&entry) {
            seen.push(entry);
        }
    }
    seen.join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_strip_shell_noise() {
        let out = format!("welcome!\n{MARKER}/a:/b{MARKER}bye");
        assert_eq!(between_markers(&out), Some("/a:/b"));
        assert_eq!(between_markers("no markers"), None);
        assert_eq!(between_markers(&format!("{MARKER}{MARKER}")), None);
    }

    #[test]
    fn merge_puts_login_first_and_dedupes() {
        let merged = merge(
            Some("/opt/homebrew/bin:/usr/bin:relative"),
            "/usr/bin:/bin:",
            Some(Path::new("/Users/me")),
        );
        let parts: Vec<&str> = merged.split(':').collect();
        assert_eq!(&parts[..3], ["/opt/homebrew/bin", "/usr/bin", "/bin"]);
        assert!(parts.contains(&"/Users/me/.local/bin"));
        assert!(parts.contains(&"/usr/local/bin"));
        assert!(!parts.contains(&"relative"));
        assert_eq!(
            parts.iter().filter(|p| **p == "/opt/homebrew/bin").count(),
            1
        );
    }

    #[test]
    fn merge_without_a_login_shell_keeps_the_current_path() {
        let merged = merge(None, "/usr/bin:/bin", None);
        assert!(merged.starts_with("/usr/bin:/bin:"));
    }
}
