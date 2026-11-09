//! Home-directory confinement for every route that touches the file system
//! on the user's behalf (agent turns, git, files, directory browsing,
//! terminals, open-in-editor). There is one check, [`HomePath`], so the
//! routes cannot drift apart.

use std::path::{Path, PathBuf};

use crate::error::AppError;

/// A path that exists and, once symlinks are resolved, lies inside the
/// user's home directory (which is canonicalized too, so a symlinked
/// `$HOME` compares correctly).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomePath {
    path: PathBuf,
    home: PathBuf,
}

/// Why a path is not a [`HomePath`].
#[derive(Debug, thiserror::Error)]
pub enum HomePathError {
    /// No path was given.
    #[error("path required")]
    Missing,
    /// The path does not exist or cannot be resolved.
    #[error("Invalid path")]
    Unresolvable(#[source] std::io::Error),
    /// The path resolves outside the home directory.
    #[error("Path must be within your home directory")]
    OutsideHome,
    /// The server cannot tell where the home directory is, so it cannot
    /// confine anything; nothing is allowed rather than everything.
    #[error("the home directory cannot be determined")]
    HomeUnknown,
    /// The home directory is set but does not resolve.
    #[error("the home directory cannot be resolved")]
    HomeUnresolvable(#[source] std::io::Error),
}

impl From<HomePathError> for AppError {
    fn from(error: HomePathError) -> Self {
        match error {
            HomePathError::Missing | HomePathError::Unresolvable(_) => {
                AppError::BadRequest(error.to_string())
            }
            HomePathError::OutsideHome => AppError::Forbidden(error.to_string()),
            HomePathError::HomeUnknown | HomePathError::HomeUnresolvable(_) => {
                AppError::internal(error)
            }
        }
    }
}

impl HomePath {
    /// Resolve `path` inside the current user's home directory.
    pub async fn resolve(path: &str) -> Result<Self, HomePathError> {
        Self::resolve_in(path, home::home_dir().as_deref()).await
    }

    /// [`HomePath::resolve`] for an optional request field; empty counts as
    /// missing.
    pub async fn required(path: Option<&str>) -> Result<Self, HomePathError> {
        match path.filter(|path| !path.is_empty()) {
            Some(path) => Self::resolve(path).await,
            None => Err(HomePathError::Missing),
        }
    }

    /// The home directory itself.
    pub async fn home() -> Result<Self, HomePathError> {
        Self::home_in(home::home_dir().as_deref()).await
    }

    /// The home directory `home` itself (`None`: the home directory is
    /// unknown).
    pub async fn home_in(home: Option<&Path>) -> Result<Self, HomePathError> {
        let home = canonical_home(home).await?;
        Ok(Self {
            path: home.clone(),
            home,
        })
    }

    /// `path` inside `home`, or `home` itself when no path is given (a chat
    /// without a project). An explicit empty path is still
    /// [`HomePathError::Missing`].
    pub async fn or_home_in(
        path: Option<&str>,
        home: Option<&Path>,
    ) -> Result<Self, HomePathError> {
        match path {
            Some(path) => Self::resolve_in(path, home).await,
            None => Self::home_in(home).await,
        }
    }

    /// Resolve `path` inside `home` (`None`: the home directory is unknown).
    pub async fn resolve_in(path: &str, home: Option<&Path>) -> Result<Self, HomePathError> {
        if path.is_empty() {
            return Err(HomePathError::Missing);
        }
        let home = canonical_home(home).await?;
        let path = tokio::fs::canonicalize(path)
            .await
            .map_err(HomePathError::Unresolvable)?;
        if !path.starts_with(&home) {
            return Err(HomePathError::OutsideHome);
        }
        Ok(Self { path, home })
    }

    pub fn as_path(&self) -> &Path {
        &self.path
    }

    pub fn into_path_buf(self) -> PathBuf {
        self.path
    }

    /// Whether this is the home directory itself.
    pub fn is_home(&self) -> bool {
        self.path == self.home
    }

    /// The parent directory, unless this is the home directory (browsing
    /// never leaves it).
    pub fn parent(&self) -> Option<&Path> {
        if self.is_home() {
            None
        } else {
            self.path.parent()
        }
    }
}

impl AsRef<Path> for HomePath {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

async fn canonical_home(home: Option<&Path>) -> Result<PathBuf, HomePathError> {
    let home = home
        .filter(|home| !home.as_os_str().is_empty())
        .ok_or(HomePathError::HomeUnknown)?;
    tokio::fs::canonicalize(home)
        .await
        .map_err(HomePathError::HomeUnresolvable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    fn status(error: HomePathError) -> StatusCode {
        AppError::from(error).into_response().status()
    }

    #[tokio::test]
    async fn paths_resolve_only_inside_the_canonical_home() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let project = home.join("project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir(root.path().join("elsewhere")).unwrap();

        let inside = HomePath::resolve_in(project.to_str().unwrap(), Some(&home))
            .await
            .unwrap();
        assert_eq!(inside.as_path(), project.canonicalize().unwrap());
        assert!(!inside.is_home());
        assert_eq!(
            inside.parent(),
            Some(home.canonicalize().unwrap().as_path())
        );

        let at_home = HomePath::resolve_in(home.to_str().unwrap(), Some(&home))
            .await
            .unwrap();
        assert!(at_home.is_home());
        assert_eq!(at_home.parent(), None);

        // `..` and symlinks are resolved before the check.
        let escape = project.join("../../elsewhere");
        let error = HomePath::resolve_in(escape.to_str().unwrap(), Some(&home))
            .await
            .unwrap_err();
        assert!(matches!(error, HomePathError::OutsideHome));
        assert_eq!(status(error), StatusCode::FORBIDDEN);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.path().join("elsewhere"), home.join("link")).unwrap();
            let error = HomePath::resolve_in(home.join("link").to_str().unwrap(), Some(&home))
                .await
                .unwrap_err();
            assert!(matches!(error, HomePathError::OutsideHome));
        }

        let error = HomePath::resolve_in(home.join("missing").to_str().unwrap(), Some(&home))
            .await
            .unwrap_err();
        assert!(matches!(error, HomePathError::Unresolvable(_)));
        assert_eq!(status(error), StatusCode::BAD_REQUEST);
        assert!(matches!(
            HomePath::resolve_in("", Some(&home)).await.unwrap_err(),
            HomePathError::Missing
        ));
    }

    /// Without a resolvable home directory nothing is allowed: the check
    /// used to be skipped, which let every path through.
    #[tokio::test]
    async fn an_unknown_home_confines_everything() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().to_str().unwrap();

        let error = HomePath::resolve_in(path, None).await.unwrap_err();
        assert!(matches!(error, HomePathError::HomeUnknown));
        assert_eq!(status(error), StatusCode::INTERNAL_SERVER_ERROR);

        let error = HomePath::resolve_in(path, Some(Path::new("")))
            .await
            .unwrap_err();
        assert!(matches!(error, HomePathError::HomeUnknown));

        let gone = root.path().join("no-such-home");
        let error = HomePath::resolve_in(path, Some(&gone)).await.unwrap_err();
        assert!(matches!(error, HomePathError::HomeUnresolvable(_)));
        assert_eq!(status(error), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
