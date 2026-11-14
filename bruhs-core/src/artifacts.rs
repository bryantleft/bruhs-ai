//! Durable self-contained HTML responses, shared by the app and provider tools.

use std::path::PathBuf;

use bruhs_models::time::EpochMs;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

mod bundle;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub title: String,
    pub html: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PreviewState {
    Ready,
    Error,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactDiagnostics {
    pub state: Option<PreviewState>,
    pub previewed_at_ms: Option<i64>,
    pub errors: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("{0}")]
    Invalid(String),
    #[error("Artifact file operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Artifact metadata could not be read: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Debug)]
pub struct ArtifactStore {
    root: PathBuf,
}

impl ArtifactStore {
    pub fn open_default() -> Result<Self, ArtifactError> {
        let root = match std::env::var_os("BRUHS_RICH_RESPONSE_DIR") {
            Some(root) => PathBuf::from(root),
            None => {
                let media = crate::uploads::media_output_directory()?;
                media
                    .parent()
                    .ok_or_else(|| {
                        ArtifactError::Invalid(
                            "The attachment directory has no application-data parent.".into(),
                        )
                    })?
                    .join("rich-responses")
            }
        };
        if !root.is_absolute() {
            return Err(ArtifactError::Invalid("BRUHS_RICH_RESPONSE_DIR must be an absolute application-data directory so the app and provider helper share the same artifacts.".into()));
        }
        Ok(Self::at(root.join("artifacts")))
    }
}

impl ArtifactStore {
    pub fn at(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn publish(
        &self,
        path: &str,
        title: Option<&str>,
        project: Option<&str>,
    ) -> Result<Artifact, ArtifactError> {
        let (html, suggested_title) = bundle::bundle(path, project)?;
        let artifact = Artifact {
            id: Uuid::new_v4().to_string(),
            title: title
                .filter(|title| !title.trim().is_empty())
                .unwrap_or(&suggested_title)
                .trim()
                .to_string(),
            html,
        };
        self.write_json(&format!("{}.json", artifact.id), &artifact)?;
        Ok(artifact)
    }

    pub fn load(&self, id: &str) -> Result<Artifact, ArtifactError> {
        validate_id(id)?;
        let artifact: Artifact =
            serde_json::from_slice(&std::fs::read(self.root.join(format!("{id}.json")))?)?;
        if artifact.id != id {
            return Err(ArtifactError::Invalid(
                "Artifact metadata does not match the requested ID.".into(),
            ));
        }
        Ok(artifact)
    }

    pub fn load_diagnostics(&self, id: &str) -> Result<ArtifactDiagnostics, ArtifactError> {
        validate_id(id)?;
        std::fs::metadata(self.root.join(format!("{id}.json")))?;
        match std::fs::read(self.root.join(format!("{id}.preview.json"))) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(ArtifactDiagnostics::default())
            }
            Err(error) => Err(error.into()),
        }
    }

    pub fn record_diagnostics(
        &self,
        id: &str,
        state: PreviewState,
        errors: Vec<String>,
    ) -> Result<ArtifactDiagnostics, ArtifactError> {
        validate_id(id)?;
        std::fs::metadata(self.root.join(format!("{id}.json")))?;
        if state == PreviewState::Ready && !errors.is_empty() {
            return Err(ArtifactError::Invalid(
                "A ready artifact preview cannot contain runtime errors.".into(),
            ));
        }
        let diagnostics = ArtifactDiagnostics {
            state: Some(state),
            previewed_at_ms: Some(EpochMs::now().as_millis()),
            errors,
        };
        self.write_json(&format!("{id}.preview.json"), &diagnostics)?;
        Ok(diagnostics)
    }

    fn write_json<T: Serialize>(&self, name: &str, value: &T) -> Result<(), ArtifactError> {
        std::fs::create_dir_all(&self.root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&self.root, std::fs::Permissions::from_mode(0o700))?;
        }
        let temp = self.root.join(format!(".{}.tmp", Uuid::new_v4()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        let result = (|| {
            serde_json::to_writer(&mut file, value)?;
            file.sync_all()?;
            std::fs::rename(&temp, self.root.join(name))?;
            #[cfg(unix)]
            std::fs::File::open(&self.root)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        result
    }
}

pub fn publish(
    path: &str,
    title: Option<&str>,
    project: Option<&str>,
) -> Result<Artifact, ArtifactError> {
    ArtifactStore::open_default()?.publish(path, title, project)
}

pub fn load(id: &str) -> Result<Artifact, ArtifactError> {
    ArtifactStore::open_default()?.load(id)
}

pub fn load_diagnostics(id: &str) -> Result<ArtifactDiagnostics, ArtifactError> {
    ArtifactStore::open_default()?.load_diagnostics(id)
}

pub fn record_diagnostics(
    id: &str,
    state: PreviewState,
    errors: Vec<String>,
) -> Result<ArtifactDiagnostics, ArtifactError> {
    ArtifactStore::open_default()?.record_diagnostics(id, state, errors)
}

fn validate_id(id: &str) -> Result<(), ArtifactError> {
    if Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id) {
        Ok(())
    } else {
        Err(ArtifactError::Invalid(
            "Artifact IDs must be canonical UUIDs.".into(),
        ))
    }
}

#[cfg(test)]
mod tests;
