//! Durable response assets and per-widget state. Client scopes are hashed;
//! user HTML cannot choose another widget's state key or filesystem path.
use bruhs_models::logic::{
    attachments::{file_extension, kind_for_name, mime_for, FileKind},
    rich_response::{media_is_remote, widget_state_key, WidgetState, MAX_WIDGET_STATE_BYTES},
};
use hmac::{Hmac, Mac};
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub mod export;
pub mod remote;

pub(crate) fn capture_turn_media(
    markdown: &str,
    project: &str,
) -> (std::collections::BTreeMap<String, String>, Vec<String>) {
    let sources = bruhs_models::logic::rich_response::all_media_sources(markdown);
    let local: Vec<_> = sources
        .into_iter()
        .filter(|source| !media_is_remote(source))
        .collect();
    if local.is_empty() {
        return (Default::default(), Vec::new());
    }
    let store = match RichStore::open_default() {
        Ok(store) => store,
        Err(error) => return (Default::default(), vec![error.to_string()]),
    };
    let mut bindings = std::collections::BTreeMap::new();
    let mut errors = Vec::new();
    for source in local {
        match store.capture(&source, Some(project)) {
            Ok(asset) => {
                bindings.insert(source, asset.src);
            }
            Err(error) => errors.push(error.to_string()),
        }
    }
    (bindings, errors)
}

#[derive(Debug, thiserror::Error)]
pub enum RichError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("Response storage failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Response data is invalid: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone)]
pub struct RichStore {
    root: PathBuf,
    media: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CapturedMedia {
    pub src: String,
    pub signed_url: String,
}

#[derive(Serialize, Deserialize)]
struct Alias {
    name: String,
}

impl RichStore {
    pub fn at(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            media: root.join("media"),
            root,
        }
    }
    pub fn open_default() -> Result<Self, RichError> {
        let media = crate::uploads::media_output_directory()?.join("media");
        let root = if let Some(path) = std::env::var_os("BRUHS_RICH_RESPONSE_DIR") {
            if path.is_empty() {
                return Err(RichError::Invalid(
                    "BRUHS_RICH_RESPONSE_DIR must not be empty.".into(),
                ));
            }
            PathBuf::from(path)
        } else {
            media
                .parent()
                .and_then(Path::parent)
                .ok_or_else(|| {
                    RichError::Invalid("Response data directory is unavailable.".into())
                })?
                .join("rich-responses")
        };
        Ok(Self { root, media })
    }
    pub fn load_state(&self, key: &str) -> Result<WidgetState, RichError> {
        key_valid(key)?;
        let path = self.root.join("state").join(format!("{key}.json"));
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(WidgetState::default())
            }
            Err(error) => return Err(error.into()),
        };
        let mut bytes = Vec::new();
        file.take(
            u64::try_from(MAX_WIDGET_STATE_BYTES).map_err(|e| RichError::Invalid(e.to_string()))?
                + 1,
        )
        .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_WIDGET_STATE_BYTES {
            return Err(RichError::Invalid(
                "Saved widget state exceeds its response-state budget.".into(),
            ));
        }
        let state: WidgetState = serde_json::from_slice(&bytes)?;
        state.validate().map_err(|e| RichError::Invalid(e.into()))?;
        Ok(state)
    }
    pub fn save_state(&self, key: &str, state: &WidgetState) -> Result<(), RichError> {
        key_valid(key)?;
        state.validate().map_err(|e| RichError::Invalid(e.into()))?;
        atomic_json(&self.root.join("state").join(format!("{key}.json")), state)
    }
    pub fn capture(&self, src: &str, project: Option<&str>) -> Result<CapturedMedia, RichError> {
        if media_is_remote(src) {
            return Err(RichError::Invalid(
                "External media requires an explicit download.".into(),
            ));
        }
        bruhs_models::logic::rich_response::validate_media_source(src)
            .map_err(|e| RichError::Invalid(e.into()))?;
        let decoded = percent_decode_str(src.strip_prefix("file://").unwrap_or(src))
            .decode_utf8()
            .map_err(|e| RichError::Invalid(e.to_string()))?;
        let alias_key = widget_state_key(project.unwrap_or("").trim_end_matches('/'), &decoded);
        let alias_path = self
            .root
            .join("media-index")
            .join(format!("{alias_key}.json"));
        let raw = Path::new(decoded.as_ref());
        let input = if raw.is_absolute() {
            raw.to_path_buf()
        } else {
            Path::new(project.filter(|p| !p.is_empty()).ok_or_else(|| {
                RichError::Invalid("Open a project to capture relative media.".into())
            })?)
            .join(raw)
        };
        if !input.exists() {
            let alias: Alias =
                serde_json::from_slice(&std::fs::read(&alias_path).map_err(|_| {
                    RichError::Invalid("This media is missing and has no saved copy.".into())
                })?)?;
            return self.describe(&alias.name);
        }
        let canonical = input.canonicalize()?;
        let already_managed = self
            .media
            .canonicalize()
            .is_ok_and(|root| canonical.starts_with(root));
        let uploaded = crate::is_upload_path(&canonical.to_string_lossy());
        let in_project = project
            .filter(|p| !p.is_empty())
            .and_then(|p| std::fs::canonicalize(p).ok())
            .is_some_and(|root| canonical.starts_with(root));
        if !already_managed && !uploaded && !in_project {
            return Err(RichError::Forbidden(
                "Response media must belong to this project or Bruhs attachments.".into(),
            ));
        }
        let ext = file_extension(&canonical.to_string_lossy()).ok_or_else(|| {
            RichError::Invalid("This media has no supported file extension.".into())
        })?;
        if !supported_media(&canonical) {
            return Err(RichError::Invalid(
                "Only images, video, audio, PDFs, and WebVTT subtitle tracks can be captured."
                    .into(),
            ));
        }
        if !canonical.is_file() {
            return Err(RichError::Invalid("This media path is not a file.".into()));
        }
        if already_managed {
            if let Some(name) = canonical.file_name().and_then(|name| name.to_str()) {
                if self.asset_path(name).is_ok() {
                    return self.describe(name);
                }
            }
        }
        private_directory(&self.media)?;
        let mut temp = tempfile::NamedTempFile::new_in(&self.media)?;
        let mut digest = Sha256::new();
        {
            let mut writer = HashWriter {
                file: temp.as_file_mut(),
                digest: &mut digest,
            };
            std::io::copy(&mut std::fs::File::open(&canonical)?, &mut writer)?;
            writer.flush()?;
        }
        temp.as_file().sync_all()?;
        let name = format!("{:x}.{ext}", digest.finalize());
        let target = self.media.join(&name);
        match temp.persist_noclobber(&target) {
            Ok(_) => {}
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.error.into()),
        }
        atomic_json(&alias_path, &Alias { name: name.clone() })?;
        self.describe(&name)
    }
    fn describe(&self, name: &str) -> Result<CapturedMedia, RichError> {
        let path = self.asset_path(name)?;
        Ok(CapturedMedia {
            src: path.to_string_lossy().into_owned(),
            signed_url: format!(
                "/api/agent/rich-response/asset/{name}?token={}",
                self.sign(name)?
            ),
        })
    }
    pub fn asset_path(&self, name: &str) -> Result<PathBuf, RichError> {
        let (id, ext) = name
            .split_once('.')
            .ok_or_else(|| RichError::Invalid("Invalid response asset name.".into()))?;
        key_valid(id)?;
        if ext.is_empty() || !ext.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(RichError::Invalid(
                "Invalid response asset extension.".into(),
            ));
        }
        let root = self.media.canonicalize()?;
        let path = root.join(name).canonicalize()?;
        if !path.starts_with(root) || !path.is_file() || !supported_media(&path) {
            return Err(RichError::Invalid(
                "This response asset is not available.".into(),
            ));
        }
        Ok(path)
    }
    pub fn authorized_asset(&self, name: &str, token: &str) -> Result<PathBuf, RichError> {
        let bytes = unhex(token)
            .ok_or_else(|| RichError::Invalid("Invalid response asset token.".into()))?;
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.secret()?)
            .map_err(|_| RichError::Invalid("Response signing key is invalid.".into()))?;
        mac.update(name.as_bytes());
        mac.verify_slice(&bytes)
            .map_err(|_| RichError::Invalid("This response asset grant is invalid.".into()))?;
        self.asset_path(name)
    }
    fn sign(&self, name: &str) -> Result<String, RichError> {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.secret()?)
            .map_err(|_| RichError::Invalid("Response signing key is invalid.".into()))?;
        mac.update(name.as_bytes());
        Ok(mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect())
    }
    fn secret(&self) -> Result<Vec<u8>, RichError> {
        private_directory(&self.root)?;
        let path = self.root.join("asset-signing-key");
        if !path.exists() {
            use rand::RngCore;
            let mut key = [0_u8; 32];
            rand::rng().fill_bytes(&mut key);
            let mut file = tempfile::NamedTempFile::new_in(&self.root)?;
            file.write_all(&key)?;
            file.as_file().sync_all()?;
            match file.persist_noclobber(&path) {
                Ok(_) => {}
                Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.error.into()),
            }
        }
        let bytes = std::fs::read(path)?;
        if bytes.len() != 32 {
            return Err(RichError::Invalid(
                "Response signing key is damaged.".into(),
            ));
        }
        Ok(bytes)
    }
}

fn supported_media(path: &Path) -> bool {
    matches!(
        kind_for_name(&path.to_string_lossy()),
        FileKind::Image | FileKind::Video | FileKind::Audio | FileKind::Pdf
    ) || file_extension(&path.to_string_lossy()).as_deref() == Some("vtt")
}
pub fn media_mime(path: &Path) -> &'static str {
    let name = path.to_string_lossy();
    if file_extension(&name).as_deref() == Some("vtt") {
        "text/vtt"
    } else {
        mime_for(kind_for_name(&name), &name)
    }
}
fn key_valid(key: &str) -> Result<(), RichError> {
    if key.len() == 64
        && key
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(RichError::Invalid(
            "Invalid response state or asset key.".into(),
        ))
    }
}
fn unhex(value: &str) -> Option<Vec<u8>> {
    if value.len() != 64 {
        return None;
    }
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| {
            u8::try_from(char::from(b[0]).to_digit(16)? * 16 + char::from(b[1]).to_digit(16)?).ok()
        })
        .collect()
}
fn private_directory(path: &Path) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(path)
    }
}
fn atomic_json(path: &Path, value: &impl Serialize) -> Result<(), RichError> {
    let parent = path
        .parent()
        .ok_or_else(|| RichError::Invalid("Invalid response storage path.".into()))?;
    private_directory(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer(file.as_file_mut(), value)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}
struct HashWriter<'a> {
    file: &'a mut std::fs::File,
    digest: &'a mut Sha256,
}
impl Write for HashWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let written = self.file.write(bytes)?;
        self.digest.update(&bytes[..written]);
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

#[cfg(test)]
mod tests;
