//! Locate, download and verify the models the daemon runs. Each
//! `WhisperModel` variant maps to its own file, URL and published SHA-256 via
//! `WhisperModel::descriptor()`; the sherpa speaker-embedding model is pinned
//! below. Models are cached under the data root so switching variants does
//! not re-download.
//!
//! A model is only ever installed by an atomic rename after its length and
//! SHA-256 match, so a native parser never loads a truncated or tampered file.
//! Concurrent requests for the same file download it once.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use bruhs_models::WhisperModel;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio_stream::StreamExt as _;

pub const SHERPA_SPEAKER_MODEL_FILENAME: &str = "3dspeaker_speech_campplus_sv_en_voxceleb_16k.onnx";
pub const SHERPA_SPEAKER_MODEL_URL: &str =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_campplus_sv_en_voxceleb_16k.onnx";
/// SHA-256 and length of [`SHERPA_SPEAKER_MODEL_URL`]. GitHub publishes no
/// digest for this release asset; the same file is mirrored on Hugging Face
/// (`lfs.oid` of `3dspeaker_speech_campplus_sv_en_voxceleb_16k.onnx` in
/// <https://huggingface.co/api/models/csukuangfj/speaker-embedding-models/tree/main>),
/// and the GitHub download was hashed to the same value on 2026-10-03.
const SHERPA_SPEAKER_MODEL_SHA256: &str =
    "357a834f702b80161e5b981182c038e18553c1f2ca752ed6cec2052365d4129b";
const SHERPA_SPEAKER_MODEL_BYTES: u64 = 29_596_978;

/// A model is a large download over a possibly slow link: no overall
/// deadline, but a stalled connection fails instead of hanging forever.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const READ_TIMEOUT: Duration = Duration::from_secs(60);
const HASH_BUFFER_BYTES: usize = 1 << 20;

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("http client unavailable: {0}")]
    Client(#[source] Arc<reqwest::Error>),
    #[error("network: {0}")]
    Network(#[from] reqwest::Error),
    #[error("http {0}")]
    Status(u16),
    #[error("download size mismatch: expected {expected} bytes, got {actual}")]
    Length { expected: u64, actual: u64 },
    #[error("download checksum mismatch: expected sha256 {expected}, got {actual}")]
    Checksum { expected: String, actual: String },
    #[error("model worker stopped: {0}")]
    Worker(#[from] tokio::task::JoinError),
}

/// One downloadable file and the bytes it must contain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelArtifact {
    /// Path under the models directory.
    pub file: PathBuf,
    pub url: String,
    /// Lowercase hex SHA-256.
    pub sha256: String,
    pub len: u64,
}

impl ModelArtifact {
    pub fn whisper(model: WhisperModel) -> Self {
        let descriptor = model.descriptor();
        Self {
            file: PathBuf::from(descriptor.filename),
            url: descriptor.url.into(),
            sha256: descriptor.sha256.into(),
            len: descriptor.size_bytes,
        }
    }

    pub fn sherpa_speaker() -> Self {
        Self {
            file: Path::new("sherpa").join(SHERPA_SPEAKER_MODEL_FILENAME),
            url: SHERPA_SPEAKER_MODEL_URL.into(),
            sha256: SHERPA_SPEAKER_MODEL_SHA256.into(),
            len: SHERPA_SPEAKER_MODEL_BYTES,
        }
    }
}

type SharedClient = OnceLock<Result<reqwest::Client, Arc<reqwest::Error>>>;

/// The model cache. One per daemon (it lives in `CaptureState`).
pub struct ModelStore {
    dir: PathBuf,
    client: SharedClient,
    /// One async lock per destination: concurrent `ensure` calls for the same
    /// file wait for a single download instead of racing.
    locks: Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>,
    /// Files hashed (or downloaded) by this process. Re-hashing a 1.6 GB model
    /// on every recording would delay its start by seconds.
    verified: Mutex<HashSet<PathBuf>>,
}

impl ModelStore {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            client: OnceLock::new(),
            locks: Mutex::new(HashMap::new()),
            verified: Mutex::new(HashSet::new()),
        }
    }

    #[cfg(test)]
    fn with_client(dir: PathBuf, client: reqwest::Client) -> Self {
        let store = Self::new(dir);
        // A fresh OnceLock is empty, so this cannot fail.
        let _ = store.client.set(Ok(client));
        store
    }

    pub fn path(&self, artifact: &ModelArtifact) -> PathBuf {
        self.dir.join(&artifact.file)
    }

    fn client(&self) -> Result<&reqwest::Client, ModelError> {
        self.client
            .get_or_init(|| {
                reqwest::Client::builder()
                    .connect_timeout(CONNECT_TIMEOUT)
                    .read_timeout(READ_TIMEOUT)
                    .build()
                    .map_err(Arc::new)
            })
            .as_ref()
            .map_err(|error| ModelError::Client(Arc::clone(error)))
    }

    fn lock_for(&self, path: &Path) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self
            .locks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Arc::clone(locks.entry(path.to_path_buf()).or_default())
    }

    fn is_verified(&self, path: &Path) -> bool {
        self.verified
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(path)
    }

    fn mark_verified(&self, path: PathBuf) {
        self.verified
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(path);
    }

    /// The Whisper model, downloading and verifying it first if needed.
    /// `progress` receives `(bytes_downloaded, total)`.
    pub async fn ensure_whisper(
        &self,
        model: WhisperModel,
        progress: impl Fn(u64, Option<u64>) + Send,
    ) -> Result<PathBuf, ModelError> {
        self.ensure(&ModelArtifact::whisper(model), progress).await
    }

    /// The sherpa speaker-embedding model (see [`Self::ensure_whisper`]).
    pub async fn ensure_sherpa_speaker(
        &self,
        progress: impl Fn(u64, Option<u64>) + Send,
    ) -> Result<PathBuf, ModelError> {
        self.ensure(&ModelArtifact::sherpa_speaker(), progress)
            .await
    }

    /// Return the cached, verified file for `artifact`, replacing a missing or
    /// corrupt one with a verified download.
    pub async fn ensure(
        &self,
        artifact: &ModelArtifact,
        progress: impl Fn(u64, Option<u64>) + Send,
    ) -> Result<PathBuf, ModelError> {
        let path = self.path(artifact);
        let lock = self.lock_for(&path);
        let _download = lock.lock().await;
        if self.is_verified(&path) && tokio::fs::try_exists(&path).await? {
            return Ok(path);
        }
        match file_matches(&path, &artifact.sha256, artifact.len).await? {
            Cached::Valid => {
                self.mark_verified(path.clone());
                return Ok(path);
            }
            Cached::Corrupt => {
                tracing::warn!(
                    "cached model {} failed verification; downloading it again",
                    path.display()
                );
                tokio::fs::remove_file(&path).await?;
            }
            Cached::Missing => {}
        }
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        download_verified(
            self.client()?,
            &artifact.url,
            &path,
            &artifact.sha256,
            artifact.len,
            progress,
        )
        .await?;
        self.mark_verified(path.clone());
        Ok(path)
    }
}

enum Cached {
    Missing,
    Corrupt,
    Valid,
}

/// Compare a cached file against its expected length and SHA-256.
async fn file_matches(path: &Path, sha256: &str, len: u64) -> Result<Cached, ModelError> {
    let path = path.to_path_buf();
    let expected = sha256.to_owned();
    Ok(
        tokio::task::spawn_blocking(move || -> std::io::Result<Cached> {
            use std::io::Read;
            let mut file = match std::fs::File::open(&path) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(Cached::Missing)
                }
                Err(error) => return Err(error),
            };
            if file.metadata()?.len() != len {
                return Ok(Cached::Corrupt);
            }
            let mut hasher = Sha256::new();
            let mut buffer = vec![0; HASH_BUFFER_BYTES];
            loop {
                let read = file.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
            }
            Ok(if hex(&hasher.finalize()) == expected {
                Cached::Valid
            } else {
                Cached::Corrupt
            })
        })
        .await??,
    )
}

/// Download `url` to `dest`. The body streams into a uniquely named temporary
/// file next to `dest`; only if its length and SHA-256 match is it renamed
/// over `dest`. On any failure the temporary file is removed and `dest` is
/// left untouched.
pub async fn download_verified(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    expected_sha256: &str,
    expected_len: u64,
    progress: impl Fn(u64, Option<u64>) + Send,
) -> Result<(), ModelError> {
    let response = client.get(url).send().await?;
    if !response.status().is_success() {
        return Err(ModelError::Status(response.status().as_u16()));
    }
    if let Some(advertised) = response.content_length() {
        if advertised != expected_len {
            return Err(ModelError::Length {
                expected: expected_len,
                actual: advertised,
            });
        }
    }
    let dir = dest
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let (file, temp_path) =
        tokio::task::spawn_blocking(move || tempfile::NamedTempFile::new_in(dir))
            .await??
            .into_parts();
    let mut file = tokio::fs::File::from_std(file);
    let written = async {
        let mut hasher = Sha256::new();
        let mut written: u64 = 0;
        let mut body = response.bytes_stream();
        while let Some(chunk) = body.next().await {
            let chunk = chunk?;
            written = written.saturating_add(chunk.len() as u64);
            if written > expected_len {
                return Err(ModelError::Length {
                    expected: expected_len,
                    actual: written,
                });
            }
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
            progress(written, Some(expected_len));
        }
        file.flush().await?;
        file.sync_all().await?;
        if written != expected_len {
            return Err(ModelError::Length {
                expected: expected_len,
                actual: written,
            });
        }
        let actual = hex(&hasher.finalize());
        if actual != expected_sha256 {
            return Err(ModelError::Checksum {
                expected: expected_sha256.to_owned(),
                actual,
            });
        }
        Ok(())
    }
    .await;
    drop(file);
    let dest = dest.to_path_buf();
    tokio::task::spawn_blocking(move || match written {
        Ok(()) => temp_path.persist(dest).map_err(|error| error.error.into()),
        Err(error) => {
            // Dropping the TempPath deletes the partial download; report the
            // download error rather than a secondary cleanup failure.
            drop(temp_path);
            Err(error)
        }
    })
    .await?
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            // Writing to a String cannot fail.
            let _ = write!(out, "{byte:02x}");
            out
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn sha256_hex(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    struct Host {
        url: String,
        hits: Arc<AtomicUsize>,
        release: Arc<tokio::sync::Semaphore>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for Host {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    /// A local model host serving `body`. Each request waits for a permit on
    /// `release`, so a test can hold a download open.
    async fn host(body: Vec<u8>, permits: usize) -> Host {
        let hits = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(tokio::sync::Semaphore::new(permits));
        let (seen, gate) = (hits.clone(), release.clone());
        let app = axum::Router::new().route(
            "/model.bin",
            axum::routing::get(move || {
                let (seen, gate, body) = (seen.clone(), gate.clone(), body.clone());
                async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                    gate.acquire().await.unwrap().forget();
                    body
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/model.bin", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Host {
            url,
            hits,
            release,
            task,
        }
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    fn artifact(url: &str, body: &[u8]) -> ModelArtifact {
        ModelArtifact {
            file: Path::new("nested").join("model.bin"),
            url: url.into(),
            sha256: sha256_hex(body),
            len: body.len() as u64,
        }
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn published_digests_are_pinned_for_every_model() {
        for model in WhisperModel::all() {
            let artifact = ModelArtifact::whisper(*model);
            assert_eq!(artifact.sha256.len(), 64, "{model:?}");
            assert!(artifact.len > 0);
            assert!(artifact.url.ends_with(artifact.file.to_str().unwrap()));
        }
        let sherpa = ModelArtifact::sherpa_speaker();
        assert_eq!(sherpa.sha256.len(), 64);
        assert_eq!(
            sherpa.file,
            Path::new("sherpa").join(SHERPA_SPEAKER_MODEL_FILENAME)
        );
    }

    #[test]
    fn model_paths_live_under_the_store_directory() {
        let root = tempfile::tempdir().unwrap();
        let store = ModelStore::new(root.path().join("models"));
        let base = store.path(&ModelArtifact::whisper(WhisperModel::BaseEn));
        assert_eq!(base, root.path().join("models/ggml-base.en.bin"));
        let small = store.path(&ModelArtifact::whisper(WhisperModel::SmallEn));
        assert_ne!(base, small);
        assert_eq!(
            store.path(&ModelArtifact::sherpa_speaker()),
            root.path()
                .join("models/sherpa")
                .join(SHERPA_SPEAKER_MODEL_FILENAME)
        );
        assert!(!root.path().join("models").exists(), "paths are pure");
    }

    #[tokio::test]
    async fn verified_download_is_installed_atomically() {
        let body = b"synthetic model weights".to_vec();
        let host = host(body.clone(), usize::MAX >> 4).await;
        let root = tempfile::tempdir().unwrap();
        let store = ModelStore::with_client(root.path().to_path_buf(), client());
        let progress = Arc::new(AtomicUsize::new(0));
        let seen = progress.clone();
        let path = store
            .ensure(&artifact(&host.url, &body), move |done, total| {
                assert_eq!(total, Some(23));
                seen.store(usize::try_from(done).unwrap(), Ordering::SeqCst);
            })
            .await
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), body);
        assert_eq!(progress.load(Ordering::SeqCst), body.len());
        assert_eq!(leftovers(path.parent().unwrap()), ["model.bin"]);
        // Cached and verified: no second request.
        store
            .ensure(&artifact(&host.url, &body), |_, _| {})
            .await
            .unwrap();
        assert_eq!(host.hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn checksum_or_length_mismatch_never_installs_the_file() {
        let body = b"tampered model weights".to_vec();
        let host = host(body.clone(), usize::MAX >> 4).await;
        let root = tempfile::tempdir().unwrap();
        let store = ModelStore::with_client(root.path().to_path_buf(), client());

        let mut wrong_hash = artifact(&host.url, &body);
        wrong_hash.sha256 = sha256_hex(b"the published weights");
        let error = store.ensure(&wrong_hash, |_, _| {}).await.unwrap_err();
        assert!(matches!(error, ModelError::Checksum { .. }), "{error}");

        let mut wrong_length = artifact(&host.url, &body);
        wrong_length.len += 1;
        let error = store.ensure(&wrong_length, |_, _| {}).await.unwrap_err();
        assert!(matches!(error, ModelError::Length { .. }), "{error}");

        let dir = root.path().join("nested");
        assert!(leftovers(&dir).is_empty(), "{:?}", leftovers(&dir));
    }

    #[tokio::test]
    async fn corrupt_cached_model_is_replaced() {
        let body = b"good weights".to_vec();
        let host = host(body.clone(), usize::MAX >> 4).await;
        let root = tempfile::tempdir().unwrap();
        let store = ModelStore::with_client(root.path().to_path_buf(), client());
        let artifact = artifact(&host.url, &body);
        let path = store.path(&artifact);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // Same length, different bytes: a partial overwrite or bit rot.
        std::fs::write(&path, b"evil weights").unwrap();
        store.ensure(&artifact, |_, _| {}).await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), body);
        assert_eq!(host.hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn concurrent_ensures_download_once() {
        let body = b"shared weights".to_vec();
        let host = host(body.clone(), 0).await;
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(ModelStore::with_client(root.path().to_path_buf(), client()));
        let artifact = artifact(&host.url, &body);
        let first = tokio::spawn({
            let (store, artifact) = (store.clone(), artifact.clone());
            async move { store.ensure(&artifact, |_, _| {}).await }
        });
        // Wait until the first download is in flight, then start the second.
        while host.hits.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        let second = tokio::spawn({
            let (store, artifact) = (store.clone(), artifact.clone());
            async move { store.ensure(&artifact, |_, _| {}).await }
        });
        host.release.add_permits(2);
        let (first, second) = (
            first.await.unwrap().unwrap(),
            second.await.unwrap().unwrap(),
        );
        assert_eq!(first, second);
        assert_eq!(std::fs::read(&first).unwrap(), body);
        assert_eq!(host.hits.load(Ordering::SeqCst), 1);
    }
}
