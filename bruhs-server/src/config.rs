//! Server configuration, read from the environment once at startup and
//! shared with handlers through the router state.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::FromRef;
use bruhs_core::updates::UpdateChecker;
use bruhs_core::AppState;

use crate::local_access::Policy;

/// Port used when `BRUHS_SERVER_PORT` is unset.
pub const DEFAULT_PORT: u16 = 3001;

#[derive(Debug, Clone)]
pub struct Config {
    /// Loopback port to listen on (`BRUHS_SERVER_PORT`).
    pub port: u16,
    /// Which browser origins may call the API (`BRUHS_API_URL`,
    /// `BRUHS_WEB_ORIGIN`).
    pub access: Policy,
    /// The built web app served for non-API paths (`BRUHS_WEB_DIST`, else
    /// `bruhs-web/dist` in this checkout).
    pub spa_dir: PathBuf,
}

/// The settings as read from the environment, before validation.
#[derive(Debug, Default, Clone, Copy)]
pub struct Settings<'a> {
    pub port: Option<&'a str>,
    pub api_url: Option<&'a str>,
    pub web_origin: Option<&'a str>,
    pub web_dist: Option<&'a str>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("BRUHS_SERVER_PORT must be a port number, got {value:?}")]
    Port {
        value: String,
        #[source]
        source: std::num::ParseIntError,
    },
    #[error("{0}")]
    Access(&'static str),
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        // Unset (or not UTF-8) reads as "use the default".
        let var = |name| std::env::var(name).ok();
        let (port, api_url, web_origin, web_dist) = (
            var("BRUHS_SERVER_PORT"),
            var("BRUHS_API_URL"),
            var("BRUHS_WEB_ORIGIN"),
            var("BRUHS_WEB_DIST"),
        );
        Self::from_settings(Settings {
            port: port.as_deref(),
            api_url: api_url.as_deref(),
            web_origin: web_origin.as_deref(),
            web_dist: web_dist.as_deref(),
        })
    }

    pub fn from_settings(settings: Settings<'_>) -> Result<Self, ConfigError> {
        let port = match settings.port.map(str::trim).filter(|p| !p.is_empty()) {
            Some(value) => value.parse().map_err(|source| ConfigError::Port {
                value: value.to_string(),
                source,
            })?,
            None => DEFAULT_PORT,
        };
        let access = Policy::from_config(settings.api_url, settings.web_origin)
            .map_err(ConfigError::Access)?;
        let spa_dir = settings
            .web_dist
            .filter(|dir| !dir.trim().is_empty())
            .map_or_else(default_spa_dir, PathBuf::from);
        Ok(Self {
            port,
            access,
            spa_dir,
        })
    }
}

/// The web app: `Contents/Resources/web` inside `Bruhs.app`, else
/// `bruhs-web/dist` next to this crate, so the server finds it whatever
/// directory it is started from.
fn default_spa_dir() -> PathBuf {
    let bundled = std::env::current_exe().ok().and_then(|exe| {
        let web = exe.parent()?.parent()?.join("Resources").join("web");
        web.is_dir().then_some(web)
    });
    if let Some(web) = bundled {
        return web;
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("bruhs-web")
        .join("dist")
}

/// Router state: the app plus the configuration, each extractable on its
/// own (`State<Arc<AppState>>`, `State<Arc<Config>>`).
#[derive(Clone)]
pub struct ServerState {
    pub app: Arc<AppState>,
    pub config: Arc<Config>,
    pub updates: UpdateChecker,
}

impl FromRef<ServerState> for UpdateChecker {
    fn from_ref(state: &ServerState) -> Self {
        state.updates.clone()
    }
}

impl FromRef<ServerState> for Arc<AppState> {
    fn from_ref(state: &ServerState) -> Self {
        Arc::clone(&state.app)
    }
}

impl FromRef<ServerState> for Arc<Config> {
    fn from_ref(state: &ServerState) -> Self {
        Arc::clone(&state.config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_overrides() {
        let config = Config::from_settings(Settings::default()).unwrap();
        assert_eq!(config.port, DEFAULT_PORT);
        assert!(config.spa_dir.ends_with("bruhs-web/dist"));
        assert!(
            config.spa_dir.starts_with(env!("CARGO_MANIFEST_DIR")),
            "found from the crate, not the working directory"
        );

        let config = Config::from_settings(Settings {
            port: Some("4100"),
            web_dist: Some("/srv/bruhs"),
            web_origin: Some("https://bruhs.localhost"),
            ..Settings::default()
        })
        .unwrap();
        assert_eq!(config.port, 4100);
        assert_eq!(config.spa_dir, PathBuf::from("/srv/bruhs"));
        assert!(config
            .access
            .allows_origin(&"https://bruhs.localhost".parse().unwrap()));
    }

    #[test]
    fn invalid_settings_are_errors() {
        for port in ["http", "70000", "-1"] {
            assert!(matches!(
                Config::from_settings(Settings {
                    port: Some(port),
                    ..Settings::default()
                }),
                Err(ConfigError::Port { .. })
            ));
        }
        assert!(matches!(
            Config::from_settings(Settings {
                api_url: Some("not an origin"),
                ..Settings::default()
            }),
            Err(ConfigError::Access(_))
        ));
    }
}
