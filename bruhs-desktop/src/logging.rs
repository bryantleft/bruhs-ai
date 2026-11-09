//! Diagnostics go to a log file in the app's data directory
//! (`logs/bruhs-desktop.log`, rotated once it passes [`ROTATE_BYTES`]), and
//! also to stderr in debug builds. `log` records from gpui are forwarded too.
//!
//! `BRUHS_DESKTOP_LOG` sets the level (`error`, `warn`, `info`, `debug`,
//! `trace`); the default is `info`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Mutex;

use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::Layer as _;

const LEVEL_ENV: &str = "BRUHS_DESKTOP_LOG";
const LOG_FILE_NAME: &str = "bruhs-desktop.log";
/// A log past this size is moved to `bruhs-desktop.log.1` at startup.
const ROTATE_BYTES: u64 = 5 * 1024 * 1024;

/// Install the global logger. Returns the log file, when one could be
/// opened (otherwise logging goes to stderr only).
pub fn init() -> Option<PathBuf> {
    let level = std::env::var(LEVEL_ENV)
        .ok()
        .and_then(|value| LevelFilter::from_str(value.trim()).ok())
        .unwrap_or(LevelFilter::INFO);
    let file = crate::paths::log_dir().and_then(|dir| match open_log_file(&dir) {
        Ok(opened) => Some(opened),
        Err(error) => {
            // No logger exists yet; stderr is the only place to say so.
            eprintln!(
                "bruhs-desktop: logging to stderr only ({}): {error}",
                dir.display()
            );
            None
        }
    });
    let path = file.as_ref().map(|(path, _)| path.clone());
    let file_layer = file.map(|(_, file)| {
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(Mutex::new(file))
            .with_filter(level)
    });
    let stderr_layer = (cfg!(debug_assertions) || path.is_none()).then(|| {
        tracing_subscriber::fmt::layer()
            .with_writer(io::stderr)
            .with_filter(level)
    });
    let installed = tracing_subscriber::registry()
        .with(file_layer)
        .with(stderr_layer)
        .try_init();
    if let Err(error) = installed {
        eprintln!("bruhs-desktop: could not install the logger: {error}");
    }
    path
}

/// Open (appending) the log in `dir`, rotating an oversized one first.
fn open_log_file(dir: &Path) -> io::Result<(PathBuf, fs::File)> {
    fs::create_dir_all(dir)?;
    let path = dir.join(LOG_FILE_NAME);
    rotate_if_large(&path, ROTATE_BYTES)?;
    let mut options = fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(&path)?;
    Ok((path, file))
}

fn rotate_if_large(path: &Path, limit: u64) -> io::Result<()> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.len() > limit => {
            let mut rotated = path.as_os_str().to_owned();
            rotated.push(".1");
            fs::rename(path, PathBuf::from(rotated))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::{open_log_file, rotate_if_large, LOG_FILE_NAME};
    use std::fs;
    use std::io::Write;

    #[test]
    fn a_large_log_is_rotated_and_a_fresh_one_appended() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOG_FILE_NAME);
        fs::write(&path, vec![b'x'; 64]).unwrap();

        rotate_if_large(&path, 32).unwrap();
        assert!(!path.exists());
        assert_eq!(
            fs::read(dir.path().join(format!("{LOG_FILE_NAME}.1")))
                .unwrap()
                .len(),
            64
        );

        let (opened, mut file) = open_log_file(dir.path()).unwrap();
        assert_eq!(opened, path);
        writeln!(file, "hello").unwrap();
        let (_, mut again) = open_log_file(dir.path()).unwrap();
        writeln!(again, "again").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello\nagain\n");
    }

    #[test]
    fn a_small_log_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOG_FILE_NAME);
        fs::write(&path, b"small").unwrap();
        rotate_if_large(&path, 32).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"small");
    }
}
