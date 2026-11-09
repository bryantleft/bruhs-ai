//! Where the daemon keeps its data. Everything lives under one injectable
//! [`DataRoot`] (on macOS `~/Library/Application Support/dev.bruhs.bruhs-ai`),
//! so tests point the daemon at a temporary directory.
//!
//! Path builders are pure: they validate the meeting identity and join path
//! components, and never touch the filesystem. Writers call
//! [`DataRoot::ensure_meeting_dir`] / [`DataRoot::ensure_audio_dir`] first.
//! Recordings, transcripts and OCR output are private: directories are 0700
//! and files 0600.

use directories::ProjectDirs;
use std::io::Write;
use std::path::{Path, PathBuf};

const QUALIFIER: &str = "dev";
const ORG: &str = "bruhs";
const APP: &str = "bruhs-ai";

/// Capture identities are opaque single path components, never caller-supplied
/// filesystem paths. Keep the same grammar at every read/write boundary.
pub fn validate_meeting_id(meeting_id: &str) -> std::io::Result<()> {
    if meeting_id.is_empty()
        || meeting_id.len() > 128
        || !meeting_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Invalid capture meeting identity.",
        ));
    }
    Ok(())
}

/// The daemon's data directories.
///
/// `data` holds audio, transcripts and models; `local` holds machine-local
/// state (the recovery queue and the per-launch auth token). The two are the
/// same directory on macOS and Linux.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataRoot {
    data: PathBuf,
    local: PathBuf,
}

impl DataRoot {
    /// The per-user application directories.
    pub fn from_project_dirs() -> std::io::Result<Self> {
        let dirs = ProjectDirs::from(QUALIFIER, ORG, APP).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "could not resolve project directories",
            )
        })?;
        Ok(Self {
            data: dirs.data_dir().to_path_buf(),
            local: dirs.data_local_dir().to_path_buf(),
        })
    }

    /// Everything under one directory (tests use a temporary one).
    #[cfg(test)]
    pub fn at(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            data: root.clone(),
            local: root,
        }
    }

    pub fn audio_dir(&self) -> PathBuf {
        self.data.join("audio")
    }

    pub fn models_dir(&self) -> PathBuf {
        self.data.join("models")
    }

    /// Machine-local daemon state (0700).
    pub fn capture_dir(&self) -> PathBuf {
        self.local.join("capture")
    }

    pub fn recovery_db_path(&self) -> PathBuf {
        self.capture_dir().join("recovery.sqlite3")
    }

    /// Default location of the per-launch bearer token. Clients find the same
    /// file through `bruhs_models::capture_auth`.
    pub fn token_path(&self) -> PathBuf {
        self.local
            .join(bruhs_models::capture_auth::TOKEN_DIR)
            .join(bruhs_models::capture_auth::TOKEN_FILE)
    }

    fn meeting_dir(&self, meeting_id: &str) -> std::io::Result<PathBuf> {
        validate_meeting_id(meeting_id)?;
        Ok(self.audio_dir().join(meeting_id))
    }

    fn meeting_file(&self, meeting_id: &str, suffix: &str) -> std::io::Result<PathBuf> {
        Ok(self
            .meeting_dir(meeting_id)?
            .join(format!("{meeting_id}.{suffix}")))
    }

    pub fn mic_wav_path(&self, meeting_id: &str) -> std::io::Result<PathBuf> {
        self.meeting_file(meeting_id, "mic.wav")
    }

    pub fn sys_wav_path(&self, meeting_id: &str) -> std::io::Result<PathBuf> {
        self.meeting_file(meeting_id, "sys.wav")
    }

    pub fn mixed_wav_path(&self, meeting_id: &str) -> std::io::Result<PathBuf> {
        self.meeting_file(meeting_id, "mixed.wav")
    }

    /// The transcript segments JSONL. It sits directly in the audio directory
    /// (not the meeting directory), where every existing transcript lives.
    pub fn segments_jsonl_path(&self, meeting_id: &str) -> std::io::Result<PathBuf> {
        validate_meeting_id(meeting_id)?;
        Ok(self.audio_dir().join(format!("{meeting_id}.jsonl")))
    }

    pub fn centroids_path(&self, meeting_id: &str) -> std::io::Result<PathBuf> {
        self.meeting_file(meeting_id, "centroids.json")
    }

    /// The OCR observations JSON written by the OCR worker.
    pub fn ocr_path(&self, meeting_id: &str) -> std::io::Result<PathBuf> {
        self.meeting_file(meeting_id, "ocr.json")
    }

    /// Create (or tighten) the audio directory before writing into it.
    pub fn ensure_audio_dir(&self) -> std::io::Result<PathBuf> {
        let dir = self.audio_dir();
        create_owned_private_dir(&dir)?;
        Ok(dir)
    }

    /// Create (or tighten) a meeting's directory before writing into it.
    pub fn ensure_meeting_dir(&self, meeting_id: &str) -> std::io::Result<PathBuf> {
        let dir = self.meeting_dir(meeting_id)?;
        self.ensure_audio_dir()?;
        create_owned_private_dir(&dir)?;
        Ok(dir)
    }

    /// Prepare the directory `token_file` is written into. The daemon's own
    /// token directory is created or tightened to 0700. Any other directory
    /// (`BRUHS_CAPTURE_TOKEN_FILE=~/token`, `/tmp/token`) belongs to the
    /// user: only its missing components are created (0700), and an existing
    /// one keeps its mode, so `$HOME` is never chmodded and a directory the
    /// user cannot chmod is not an error.
    pub fn ensure_token_dir(&self, token_file: &Path) -> std::io::Result<()> {
        let Some(dir) = token_file
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
        else {
            return Ok(());
        };
        if self.token_path().parent() == Some(dir) {
            create_owned_private_dir(dir)
        } else {
            create_private_dir_all(dir)
        }
    }
}

/// Create `dir` and any missing parents owner-only (0700). Directories that
/// already exist keep their mode: this may be a path the user named, such as
/// a directory in `$HOME`.
pub fn create_private_dir_all(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// [`create_private_dir_all`], then tighten `dir` itself to 0700 in case an
/// earlier build created it world-readable. Only for the daemon's own
/// app-data directories.
fn create_owned_private_dir(dir: &Path) -> std::io::Result<()> {
    create_private_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Open `path` for writing (create or truncate), readable only by the owner.
pub fn create_private_file(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    // `mode` only applies when the file is created; tighten a reused file too.
    restrict_to_owner(&file)?;
    Ok(file)
}

/// Open `path` for appending (creating it if needed), owner-only.
pub fn append_private_file(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// Replace `path` with `bytes` atomically (temp file + rename), owner-only.
pub fn write_private_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    // NamedTempFile is created 0600 on unix and keeps that mode on rename.
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn restrict_to_owner(file: &std::fs::File) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = file;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn meeting_identity_rejects_paths_and_accepts_existing_opaque_ids() {
        for invalid in [
            "",
            ".",
            "..",
            "../outside",
            "/absolute",
            "a/b",
            "a\\b",
            "a\0b",
            "a b",
            "a.jsonl",
            "%2f",
            "é",
            "🔒",
        ] {
            assert_eq!(
                validate_meeting_id(invalid).unwrap_err().kind(),
                std::io::ErrorKind::InvalidInput,
                "{invalid:?}"
            );
        }
        assert!(validate_meeting_id(&"a".repeat(129)).is_err());
        for valid in [
            "existing-meeting_01",
            "ABC123",
            "00000000-0000-0000-0000-000000000000",
        ] {
            validate_meeting_id(valid).unwrap();
        }
        validate_meeting_id(&"a".repeat(128)).unwrap();
    }

    #[test]
    fn every_public_capture_path_rejects_traversal_before_filesystem_access() {
        let root = tempfile::tempdir().unwrap();
        let data = DataRoot::at(root.path().join("data"));
        let outside = root.path().join("must-not-be-created");
        let absolute = outside.to_str().unwrap();
        let constructors: &[fn(&DataRoot, &str) -> std::io::Result<PathBuf>] = &[
            DataRoot::mic_wav_path,
            DataRoot::sys_wav_path,
            DataRoot::mixed_wav_path,
            DataRoot::segments_jsonl_path,
            DataRoot::centroids_path,
            DataRoot::ocr_path,
            DataRoot::ensure_meeting_dir,
        ];
        for constructor in constructors {
            let error = constructor(&data, absolute).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
            assert!(
                !outside.exists(),
                "path constructor created an absolute caller path"
            );
        }
        assert!(!root.path().join("data").exists());
    }

    /// Path builders used to `create_dir_all` as a side effect, which is how
    /// tests (and `GET /ocr`, `GET /centroids`) littered the real audio dir.
    #[test]
    fn path_builders_are_pure() {
        let root = tempfile::tempdir().unwrap();
        let data = DataRoot::at(root.path().join("data"));
        let id = "synthetic-meeting";
        let paths = [
            data.mic_wav_path(id).unwrap(),
            data.sys_wav_path(id).unwrap(),
            data.mixed_wav_path(id).unwrap(),
            data.segments_jsonl_path(id).unwrap(),
            data.centroids_path(id).unwrap(),
            data.ocr_path(id).unwrap(),
        ];
        for path in &paths {
            assert!(path.starts_with(root.path().join("data").join("audio")));
            assert!(path.to_string_lossy().contains(id));
        }
        assert!(!root.path().join("data").exists(), "no directory created");
    }

    #[test]
    fn meeting_files_keep_their_existing_names() {
        let data = DataRoot::at("/synthetic-root");
        let audio = Path::new("/synthetic-root/audio");
        assert_eq!(
            data.mic_wav_path("m-1").unwrap(),
            audio.join("m-1/m-1.mic.wav")
        );
        assert_eq!(
            data.sys_wav_path("m-1").unwrap(),
            audio.join("m-1/m-1.sys.wav")
        );
        assert_eq!(
            data.mixed_wav_path("m-1").unwrap(),
            audio.join("m-1/m-1.mixed.wav")
        );
        assert_eq!(
            data.segments_jsonl_path("m-1").unwrap(),
            audio.join("m-1.jsonl")
        );
        assert_eq!(
            data.centroids_path("m-1").unwrap(),
            audio.join("m-1/m-1.centroids.json")
        );
        assert_eq!(
            data.ocr_path("m-1").unwrap(),
            audio.join("m-1/m-1.ocr.json")
        );
        assert_eq!(
            data.recovery_db_path(),
            Path::new("/synthetic-root/capture/recovery.sqlite3")
        );
        assert_eq!(data.models_dir(), Path::new("/synthetic-root/models"));
        assert_eq!(
            data.token_path(),
            Path::new("/synthetic-root/capture/daemon-token")
        );
    }

    #[test]
    fn default_root_is_the_app_support_directory() {
        let root = DataRoot::from_project_dirs().expect("ProjectDirs should resolve");
        assert!(root.audio_dir().is_absolute());
        assert!(root.audio_dir().ends_with("audio"));
    }

    #[cfg(unix)]
    #[test]
    fn writers_create_private_directories_and_files() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let data = DataRoot::at(root.path());
        // An audio directory left world-readable by an earlier build is tightened.
        std::fs::create_dir(data.audio_dir()).unwrap();
        std::fs::set_permissions(data.audio_dir(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let dir = data.ensure_meeting_dir("m-1").unwrap();
        assert_eq!(mode(&data.audio_dir()), 0o700);
        assert_eq!(mode(&dir), 0o700);

        let wav = data.mic_wav_path("m-1").unwrap();
        drop(create_private_file(&wav).unwrap());
        assert_eq!(mode(&wav), 0o600);
        // A reused file is tightened too.
        std::fs::set_permissions(&wav, std::fs::Permissions::from_mode(0o644)).unwrap();
        drop(create_private_file(&wav).unwrap());
        assert_eq!(mode(&wav), 0o600);

        let jsonl = data.segments_jsonl_path("m-1").unwrap();
        drop(append_private_file(&jsonl).unwrap());
        assert_eq!(mode(&jsonl), 0o600);

        let ocr = data.ocr_path("m-1").unwrap();
        write_private_atomic(&ocr, b"[]").unwrap();
        assert_eq!(mode(&ocr), 0o600);
        assert_eq!(std::fs::read(&ocr).unwrap(), b"[]");
    }
}
