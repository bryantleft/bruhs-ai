//! WSL backend discovery, configuration, and lifecycle management (Windows hosts).
use bruhs_models::environment_parity::{
    WslAction, WslBackendConfig, WslBackendStatus, WslDistro, WslDistroState,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum WslError {
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Command(String),
    #[error("WSL config storage failed: {0}")]
    Storage(#[from] std::io::Error),
    #[error("WSL config is corrupt: {0}")]
    Json(#[from] serde_json::Error),
}

pub type WslResult<T> = Result<T, WslError>;

/// Decode raw stdout bytes from `wsl.exe --list --verbose` (which defaults to UTF-16LE on Windows).
#[must_use]
pub fn decode_wsl_output(bytes: &[u8]) -> String {
    let stripped = bytes.strip_prefix(&[0xFF, 0xFE]).unwrap_or(bytes);
    if stripped.len() >= 2
        && stripped.len().is_multiple_of(2)
        && stripped.iter().skip(1).step_by(2).any(|b| *b == 0)
    {
        let units: Vec<u16> = stripped
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| u16::from_le_bytes(pair))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// Parse `wsl.exe --list --verbose` output into structured distributions.
#[must_use]
pub fn parse_wsl_list_verbose(output: &str) -> Vec<WslDistro> {
    let mut distros = Vec::new();
    for raw_line in output.lines() {
        let line = raw_line.trim_matches(|c: char| c == '\u{feff}' || c == '\r' || c == '\0');
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let is_default = trimmed.starts_with('*');
        let rest = trimmed.trim_start_matches('*').trim();
        let tokens: Vec<&str> = rest.split_whitespace().collect();
        if tokens.len() < 3 {
            continue;
        }
        // Skip header line ("NAME STATE VERSION").
        if tokens[0].eq_ignore_ascii_case("NAME") && tokens[1].eq_ignore_ascii_case("STATE") {
            continue;
        }
        let Ok(version) = tokens[tokens.len() - 1].parse::<u8>() else {
            continue;
        };
        let state = match tokens[tokens.len() - 2].to_ascii_lowercase().as_str() {
            "running" => WslDistroState::Running,
            "stopped" => WslDistroState::Stopped,
            "installing" => WslDistroState::Installing,
            _ => WslDistroState::Unknown,
        };
        let name = tokens[..tokens.len() - 2].join(" ");
        if name.is_empty() {
            continue;
        }
        distros.push(WslDistro {
            name,
            state,
            version,
            is_default,
        });
    }
    distros
}

struct Inner {
    path: PathBuf,
    config: Mutex<WslBackendConfig>,
}

/// Durable WSL backend settings and host lifecycle controller.
#[derive(Clone)]
pub struct WslManager(Arc<Inner>);

impl WslManager {
    pub fn default_path() -> std::io::Result<PathBuf> {
        Ok(crate::agent_extensions::directory()?.join("wsl-backend.json"))
    }

    pub async fn open(path: PathBuf) -> WslResult<Self> {
        let config = tokio::task::spawn_blocking({
            let path = path.clone();
            move || load_config(&path)
        })
        .await
        .map_err(|err| std::io::Error::other(err.to_string()))??;
        Ok(Self(Arc::new(Inner {
            path,
            config: Mutex::new(config),
        })))
    }

    pub async fn status(&self) -> WslBackendStatus {
        let config = self.0.config.lock().await.clone();
        #[cfg(target_os = "windows")]
        {
            match probe_windows_wsl().await {
                Ok(distros) => {
                    let running_distro = distros
                        .iter()
                        .find(|d| d.state == WslDistroState::Running)
                        .map(|d| d.name.clone());
                    WslBackendStatus {
                        platform_supported: true,
                        available: !distros.is_empty(),
                        running_distro,
                        distros,
                        config,
                        diagnostic: None,
                    }
                }
                Err(detail) => WslBackendStatus {
                    platform_supported: true,
                    available: false,
                    running_distro: None,
                    distros: Vec::new(),
                    config,
                    diagnostic: Some(detail),
                },
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            WslBackendStatus {
                platform_supported: false,
                available: false,
                running_distro: None,
                distros: Vec::new(),
                config,
                diagnostic: Some("WSL backend management is only available on Windows.".into()),
            }
        }
    }

    pub async fn update_config(&self, mut next: WslBackendConfig) -> WslResult<WslBackendStatus> {
        if let Some(distro) = &mut next.preferred_distro {
            let trimmed = distro.trim().to_owned();
            if trimmed.chars().any(char::is_control) {
                return Err(WslError::Invalid("Invalid WSL distribution name.".into()));
            }
            if trimmed.is_empty() {
                next.preferred_distro = None;
            } else {
                *distro = trimmed;
            }
        }
        {
            let mut guard = self.0.config.lock().await;
            *guard = next;
            let res = persist_config(&self.0.path, &guard);
            drop(guard);
            res?;
        }
        Ok(self.status().await)
    }

    pub async fn control(
        &self,
        action: WslAction,
        distro: Option<String>,
    ) -> WslResult<WslBackendStatus> {
        #[cfg(not(target_os = "windows"))]
        {
            let _ = (action, distro);
            Err(WslError::Unsupported(
                "WSL backend control is only supported on Windows hosts.".into(),
            ))
        }
        #[cfg(target_os = "windows")]
        {
            let target = match distro {
                Some(name) if !name.trim().is_empty() => name.trim().to_owned(),
                _ => self
                    .0
                    .config
                    .lock()
                    .await
                    .preferred_distro
                    .clone()
                    .ok_or_else(|| WslError::Invalid("Choose a WSL distribution first.".into()))?,
            };
            if target.starts_with('-') || target.chars().any(char::is_control) {
                return Err(WslError::Invalid("Invalid WSL distribution name.".into()));
            }
            run_windows_wsl_action(action, &target).await?;
            Ok(self.status().await)
        }
    }
}

#[cfg(target_os = "windows")]
async fn probe_windows_wsl() -> Result<Vec<WslDistro>, String> {
    let out = tokio::process::Command::new("wsl.exe")
        .args(["--list", "--verbose"])
        .output()
        .await
        .map_err(|err| format!("wsl.exe unavailable: {err}"))?;
    if !out.status.success() {
        return Err(decode_wsl_output(&out.stderr).trim().to_owned());
    }
    Ok(parse_wsl_list_verbose(&decode_wsl_output(&out.stdout)))
}

#[cfg(target_os = "windows")]
async fn run_windows_wsl_action(action: WslAction, distro: &str) -> WslResult<()> {
    let run = |args: &[&str]| async {
        let out = tokio::process::Command::new("wsl.exe")
            .args(args)
            .output()
            .await
            .map_err(|err| WslError::Command(format!("Failed to invoke wsl.exe: {err}")))?;
        if !out.status.success() {
            let err = decode_wsl_output(&out.stderr);
            return Err(WslError::Command(if err.trim().is_empty() {
                format!("wsl.exe exited with {}", out.status)
            } else {
                err.trim().to_owned()
            }));
        }
        Ok(())
    };
    match action {
        WslAction::Start => run(&["-d", distro, "--", "true"]).await,
        WslAction::Stop => run(&["--terminate", distro]).await,
        WslAction::Restart => {
            run(&["--terminate", distro]).await?;
            run(&["-d", distro, "--", "true"]).await
        }
    }
}

fn load_config(path: &Path) -> WslResult<WslBackendConfig> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(WslBackendConfig::default()),
        Err(err) => Err(WslError::Storage(err)),
    }
}

fn persist_config(path: &Path, config: &WslBackendConfig) -> WslResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("json.{}.tmp", Uuid::new_v4()));
    let bytes = serde_json::to_vec_pretty(config)?;
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_utf16_and_utf8_wsl_verbose_output() {
        let sample = "  NAME            STATE           VERSION\r\n* Ubuntu-24.04    Running         2\r\n  Debian          Stopped         2\r\n";
        let utf16: Vec<u8> = std::iter::once(0xFEFFu16)
            .chain(sample.encode_utf16())
            .flat_map(u16::to_le_bytes)
            .collect();
        let decoded = decode_wsl_output(&utf16);
        let parsed = parse_wsl_list_verbose(&decoded);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "Ubuntu-24.04");
        assert_eq!(parsed[0].state, WslDistroState::Running);
        assert_eq!(parsed[0].version, 2);
        assert!(parsed[0].is_default);
        assert_eq!(parsed[1].name, "Debian");
        assert_eq!(parsed[1].state, WslDistroState::Stopped);
        assert!(!parsed[1].is_default);
    }

    #[tokio::test]
    async fn wsl_manager_persists_config_and_reports_platform_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wsl.json");
        let mgr = WslManager::open(path.clone()).await.unwrap();
        let updated = mgr
            .update_config(WslBackendConfig {
                enabled: true,
                preferred_distro: Some("Ubuntu-24.04".into()),
                stop_windows_backend: true,
            })
            .await
            .unwrap();
        assert_eq!(
            updated.config.preferred_distro.as_deref(),
            Some("Ubuntu-24.04")
        );
        assert!(updated.config.stop_windows_backend);

        let reopened = WslManager::open(path).await.unwrap();
        let status = reopened.status().await;
        assert_eq!(
            status.config.preferred_distro.as_deref(),
            Some("Ubuntu-24.04")
        );
        #[cfg(not(target_os = "windows"))]
        {
            assert!(!status.platform_supported);
            assert!(matches!(
                reopened.control(WslAction::Start, None).await,
                Err(WslError::Unsupported(_))
            ));
        }
    }
}
