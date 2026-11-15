//! Run bruhs-server as a login service (launchd on macOS, a systemd user unit
//! on Linux) so reactive agents keep working after the app quits.
//!
//! The service starts this same binary with the `BRUHS_*` settings and `PATH`
//! of the process that installed it (agents need `PATH` to find their CLIs).
//! It restarts only after a failure: while the app's own server holds the
//! port and the automation database, the service's start fails and is retried
//! every 30 seconds, so it takes over once the app quits.

use bruhs_models::api::{BackgroundService, BackgroundServiceAction};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

const LABEL: &str = "dev.bruhs.server";
const UNIT: &str = "bruhs-server.service";
/// Settings that describe this process's own lifetime, not the service's.
const SKIPPED_ENV: [&str; 1] = ["BRUHS_EXIT_WITH_PARENT"];

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    Failed(String),
}

fn home() -> Result<PathBuf, ServiceError> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| ServiceError::Failed("HOME is not set.".into()))
}

fn definition_path() -> Result<PathBuf, ServiceError> {
    if cfg!(target_os = "macos") {
        Ok(home()?
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist")))
    } else if cfg!(target_os = "linux") {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .map_or_else(|| home().map(|home| home.join(".config")), Ok)?;
        Ok(config.join("systemd/user").join(UNIT))
    } else {
        Err(ServiceError::Unsupported(
            "Background service install supports macOS and Linux.".into(),
        ))
    }
}

/// Environment carried into the service, sorted for a stable definition.
fn environment(web_dist: &Path) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = std::env::vars()
        .filter(|(key, _)| {
            (key.starts_with("BRUHS_") && !SKIPPED_ENV.contains(&key.as_str())) || key == "PATH"
        })
        .collect();
    env.retain(|(key, _)| key != "BRUHS_WEB_DIST");
    env.push(("BRUHS_WEB_DIST".into(), web_dist.display().to_string()));
    env.sort();
    env
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub(crate) fn launchd_plist(binary: &Path, env: &[(String, String)], log_dir: &Path) -> String {
    let variables = env
        .iter()
        .map(|(key, value)| {
            format!(
                "\t\t<key>{}</key>\n\t\t<string>{}</string>\n",
                xml(key),
                xml(value)
            )
        })
        .collect::<String>();
    let log = |name: &str| xml(&log_dir.join(name).display().to_string());
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n\t<key>Label</key>\n\t<string>{LABEL}</string>\n\t<key>ProgramArguments</key>\n\t<array>\n\t\t<string>{}</string>\n\t</array>\n\t<key>EnvironmentVariables</key>\n\t<dict>\n{variables}\t</dict>\n\t<key>RunAtLoad</key>\n\t<true/>\n\t<key>KeepAlive</key>\n\t<dict>\n\t\t<key>SuccessfulExit</key>\n\t\t<false/>\n\t</dict>\n\t<key>ThrottleInterval</key>\n\t<integer>30</integer>\n\t<key>ProcessType</key>\n\t<string>Background</string>\n\t<key>StandardOutPath</key>\n\t<string>{}</string>\n\t<key>StandardErrorPath</key>\n\t<string>{}</string>\n</dict>\n</plist>\n",
        xml(&binary.display().to_string()),
        log("bruhs-server.service.log"),
        log("bruhs-server.service.log"),
    )
}

fn systemd_quote(text: &str) -> String {
    format!(
        "\"{}\"",
        text.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
    )
}

pub(crate) fn systemd_unit(binary: &Path, env: &[(String, String)]) -> String {
    let variables = env
        .iter()
        .map(|(key, value)| format!("Environment={}\n", systemd_quote(&format!("{key}={value}"))))
        .collect::<String>();
    format!(
        "[Unit]\nDescription=bruhs local server and automations\nAfter=network-online.target\n\n[Service]\nExecStart={}\n{variables}Restart=on-failure\nRestartSec=30\n\n[Install]\nWantedBy=default.target\n",
        systemd_quote(&binary.display().to_string())
    )
}

fn run(program: &str, args: &[&str]) -> Result<String, ServiceError> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| ServiceError::Failed(format!("Could not run {program}: {error}")))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(ServiceError::Failed(format!(
            "{program} {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

fn gui_domain() -> Result<String, ServiceError> {
    Ok(format!("gui/{}", run("id", &["-u"])?))
}

fn write_private(path: &Path, contents: &str) -> Result<(), ServiceError> {
    let failed = |error: std::io::Error| {
        ServiceError::Failed(format!("Could not write {}: {error}", path.display()))
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(failed)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // The definition can carry configured BRUHS_* credentials.
        options.mode(0o600);
    }
    use std::io::Write;
    options
        .open(path)
        .and_then(|mut file| file.write_all(contents.as_bytes()))
        .map_err(failed)
}

fn log_dir() -> Result<PathBuf, ServiceError> {
    directories::ProjectDirs::from("dev", "bruhs", "bruhs-ai")
        .map(|dirs| dirs.data_local_dir().join("logs"))
        .ok_or_else(|| ServiceError::Failed("Could not find the app data directory.".into()))
}

pub fn status() -> BackgroundService {
    let path = match definition_path() {
        Ok(path) => path,
        Err(error) => {
            return BackgroundService {
                detail: error.to_string(),
                ..BackgroundService::default()
            }
        }
    };
    let installed = path.is_file();
    let loaded = installed
        && if cfg!(target_os = "macos") {
            gui_domain()
                .and_then(|domain| run("launchctl", &["print", &format!("{domain}/{LABEL}")]))
                .is_ok()
        } else {
            run("systemctl", &["--user", "is-active", "--quiet", UNIT]).is_ok()
        };
    let binary = installed.then(|| installed_binary(&path)).flatten();
    BackgroundService {
        supported: true,
        installed,
        loaded,
        detail: match (installed, loaded) {
            (false, _) => "Automations stop when this server stops.".into(),
            (true, true) => {
                "Automations keep running after the app quits and start at login.".into()
            }
            (true, false) => {
                "Installed, waiting to start. It takes over once the running server exits.".into()
            }
        },
        binary,
    }
}

fn installed_binary(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    if cfg!(target_os = "macos") {
        let after = text.split("<key>ProgramArguments</key>").nth(1)?;
        let start = after.find("<string>")? + "<string>".len();
        let end = after[start..].find("</string>")? + start;
        Some(
            after[start..end]
                .replace("&amp;", "&")
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&quot;", "\""),
        )
    } else {
        text.lines()
            .find_map(|line| line.strip_prefix("ExecStart="))
            .map(|value| value.trim_matches('"').to_owned())
    }
}

pub fn apply(
    action: BackgroundServiceAction,
    web_dist: &Path,
) -> Result<BackgroundService, ServiceError> {
    let path = definition_path()?;
    match action {
        BackgroundServiceAction::Install => {
            let binary = std::env::current_exe()
                .and_then(std::fs::canonicalize)
                .map_err(|error| {
                    ServiceError::Failed(format!("Could not find this server binary: {error}"))
                })?;
            let env = environment(web_dist);
            if cfg!(target_os = "macos") {
                let logs = log_dir()?;
                std::fs::create_dir_all(&logs)
                    .map_err(|error| ServiceError::Failed(error.to_string()))?;
                let domain = gui_domain()?;
                // Replace any earlier definition; an absent service is fine.
                let _ = run("launchctl", &["bootout", &format!("{domain}/{LABEL}")]);
                write_private(&path, &launchd_plist(&binary, &env, &logs))?;
                run(
                    "launchctl",
                    &["bootstrap", &domain, &path.display().to_string()],
                )?;
            } else {
                write_private(&path, &systemd_unit(&binary, &env))?;
                run("systemctl", &["--user", "daemon-reload"])?;
                run("systemctl", &["--user", "enable", "--now", UNIT])?;
            }
        }
        BackgroundServiceAction::Uninstall => {
            if cfg!(target_os = "macos") {
                let domain = gui_domain()?;
                let _ = run("launchctl", &["bootout", &format!("{domain}/{LABEL}")]);
            } else {
                let _ = run("systemctl", &["--user", "disable", "--now", UNIT]);
            }
            if path.exists() {
                std::fs::remove_file(&path)
                    .map_err(|error| ServiceError::Failed(error.to_string()))?;
            }
            if cfg!(target_os = "linux") {
                let _ = run("systemctl", &["--user", "daemon-reload"]);
            }
        }
    }
    Ok(status())
}

/// `bruhs-server service install|uninstall|status`. Returns an exit code when handled.
pub fn cli(web_dist: &Path) -> Option<i32> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("service") {
        return None;
    }
    let result = match args.next().as_deref() {
        Some("install") => apply(BackgroundServiceAction::Install, web_dist),
        Some("uninstall") => apply(BackgroundServiceAction::Uninstall, web_dist),
        Some("status") | None => Ok(status()),
        Some(other) => {
            eprintln!("Unknown service command “{other}”. Use install, uninstall or status.");
            return Some(2);
        }
    };
    match result {
        Ok(status) => {
            println!("{}", status.detail);
            if let Some(binary) = status.binary {
                println!("Binary: {binary}");
            }
            Some(0)
        }
        Err(error) => {
            eprintln!("{error}");
            Some(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions_escape_values_and_restart_only_on_failure() {
        let env = vec![
            ("BRUHS_WEB_DIST".to_owned(), "/Apps/<b&r>/web".to_owned()),
            ("PATH".to_owned(), "/usr/bin:/opt/100%".to_owned()),
        ];
        let plist = launchd_plist(
            Path::new("/Apps/bruhs \"x\"/bruhs-server"),
            &env,
            Path::new("/logs"),
        );
        assert!(plist.contains("<string>/Apps/&lt;b&amp;r&gt;/web</string>"));
        assert!(plist.contains("<string>/Apps/bruhs &quot;x&quot;/bruhs-server</string>"));
        assert!(plist.contains("<key>SuccessfulExit</key>\n\t\t<false/>"));
        let unit = systemd_unit(Path::new("/opt/bruhs/bruhs-server"), &env);
        assert!(unit.contains("Environment=\"PATH=/usr/bin:/opt/100%%\""));
        assert!(unit.contains("Restart=on-failure"));
    }

    #[test]
    fn the_installer_carries_settings_but_not_its_own_lifetime() {
        let env = environment(Path::new("/web"));
        assert!(env.iter().all(|(key, _)| key != "BRUHS_EXIT_WITH_PARENT"));
        assert!(env
            .iter()
            .any(|(key, value)| key == "BRUHS_WEB_DIST" && value == "/web"));
    }
}
