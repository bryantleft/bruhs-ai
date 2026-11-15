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

/// Who is asking for the status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asker {
    /// A running server answering `/api/service`; it is the process on the
    /// port, so it never probes itself.
    Server { pid: u32 },
    /// The `service` command, which probes the port once.
    Cli { port: u16 },
}

/// What answered a readiness probe on the port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Responder {
    /// Nothing answered in time.
    Silent,
    /// Something answered without saying which process it is.
    Unidentified,
    /// A bruhs-server that named its process.
    Process(u32),
}

pub fn status(asker: Asker) -> BackgroundService {
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
    let manager = installed.then(manager_state).flatten();
    let loaded = manager.is_some();
    let service_pid = manager.flatten();
    let binary = installed.then(|| installed_binary(&path)).flatten();
    let log_path = installed
        .then(service_log)
        .flatten()
        .map(|path| path.display().to_string());
    let responder = match asker {
        Asker::Server { pid } => Responder::Process(pid),
        // Only worth a probe when the service has a process to find.
        Asker::Cli { port } if service_pid.is_some() => probe(port),
        Asker::Cli { .. } => Responder::Silent,
    };
    let readiness = readiness(installed, loaded, service_pid, responder);
    BackgroundService {
        supported: true,
        installed,
        loaded,
        binary,
        detail: readiness.detail(log_path.as_deref(), asker),
        serving: readiness == Readiness::Serving,
        service_pid,
        responder_pid: match responder {
            Responder::Process(pid) => Some(pid),
            _ => None,
        },
        log_path,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Readiness {
    NotInstalled,
    /// Installed, but the service manager has not loaded it.
    Waiting,
    /// Loaded without a running process (failed start, retrying).
    NotRunning,
    /// The service has a process, but another server holds the port.
    PortHeld,
    /// The service has a process, but nothing answers on the port.
    Silent,
    /// Something answers, but it does not identify itself.
    Unverified,
    Serving,
}

/// Serving needs the service's own nonzero process to be the one answering;
/// a missing, zero or different process never counts.
fn readiness(
    installed: bool,
    loaded: bool,
    service_pid: Option<u32>,
    responder: Responder,
) -> Readiness {
    if !installed {
        return Readiness::NotInstalled;
    }
    if !loaded {
        return Readiness::Waiting;
    }
    let Some(service) = service_pid.filter(|pid| *pid != 0) else {
        return Readiness::NotRunning;
    };
    match responder {
        Responder::Process(pid) if pid != 0 && pid == service => Readiness::Serving,
        Responder::Process(_) => Readiness::PortHeld,
        Responder::Unidentified => Readiness::Unverified,
        Responder::Silent => Readiness::Silent,
    }
}

impl Readiness {
    fn detail(self, log: Option<&str>, asker: Asker) -> String {
        let see_log = log.map_or_else(String::new, |log| format!(" Its log is at {log}."));
        let address = match asker {
            Asker::Cli { port } => format!("127.0.0.1:{port}"),
            Asker::Server { .. } => "its port".into(),
        };
        match self {
            Self::NotInstalled => "Automations stop when this server stops.".into(),
            Self::Waiting => {
                "Installed, waiting to start. It takes over once the running server exits.".into()
            }
            Self::NotRunning => match asker {
                // Expected while this server holds the port.
                Asker::Server { .. } => {
                    "Installed. The service starts once this server exits.".into()
                }
                Asker::Cli { .. } => format!(
                    "Installed, but the service is not running. It retries every 30 seconds.{see_log}"
                ),
            },
            Self::PortHeld => match asker {
                Asker::Server { .. } => {
                    "Installed. The service takes over once this server exits.".into()
                }
                Asker::Cli { .. } => format!(
                    "Installed, but another server holds {address}. The service takes over once it exits."
                ),
            },
            Self::Silent => format!(
                "Installed and started, but it is not answering on {address}.{see_log}"
            ),
            Self::Unverified => format!(
                "Installed and started, but its readiness is unverified: something answers on {address} without naming its process.{see_log}"
            ),
            Self::Serving => {
                "Automations keep running after the app quits and start at login.".into()
            }
        }
    }
}

/// `None` when the manager does not have the service loaded; otherwise the
/// loaded state with its process, if it reports one.
fn manager_state() -> Option<Option<u32>> {
    if cfg!(target_os = "macos") {
        let domain = gui_domain().ok()?;
        let printed = run("launchctl", &["print", &format!("{domain}/{LABEL}")]).ok()?;
        Some(launchd_pid(&printed))
    } else {
        run("systemctl", &["--user", "is-active", "--quiet", UNIT]).ok()?;
        let main = run(
            "systemctl",
            &["--user", "show", "--property=MainPID", "--value", UNIT],
        )
        .ok();
        Some(main.and_then(|text| positive_pid(text.trim())))
    }
}

/// The job's own `pid = N` line from `launchctl print` (the first one; nested
/// sections come later).
fn launchd_pid(printed: &str) -> Option<u32> {
    printed
        .lines()
        .find_map(|line| line.trim().strip_prefix("pid = "))
        .and_then(positive_pid)
}

fn positive_pid(text: &str) -> Option<u32> {
    text.parse::<u32>().ok().filter(|pid| *pid != 0)
}

fn service_log() -> Option<PathBuf> {
    cfg!(target_os = "macos")
        .then(|| log_dir().ok())
        .flatten()
        .map(|dir| dir.join("bruhs-server.service.log"))
}

/// One bounded request to the loopback port. Only a well-formed status that
/// names its process identifies the responder.
fn probe(port: u16) -> Responder {
    probe_within(
        std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        PROBE_BUDGET,
    )
}

/// The whole probe (connect, request and every read) shares one deadline, so
/// a responder that drips bytes or keeps the connection open cannot hold the
/// command past `budget`; `PROBE_LIMIT` separately caps the bytes kept.
fn probe_within(address: std::net::SocketAddr, budget: std::time::Duration) -> Responder {
    use std::io::{ErrorKind, Read, Write};
    use std::net::TcpStream;
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + budget;
    let remaining = || {
        deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
    };
    let Some(left) = remaining() else {
        return Responder::Silent;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&address, left.min(Duration::from_secs(1)))
    else {
        return Responder::Silent;
    };
    let Some(left) = remaining() else {
        return Responder::Silent;
    };
    let port = address.port();
    let request = format!(
        "GET /api/service HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
    );
    if stream.set_write_timeout(Some(left)).is_err()
        || stream.write_all(request.as_bytes()).is_err()
    {
        return Responder::Silent;
    }
    let mut response = Vec::new();
    let mut buffer = [0_u8; 4096];
    while response.len() < PROBE_LIMIT {
        // Each read gets only what is left of the shared deadline.
        let Some(left) = remaining() else { break };
        if stream.set_read_timeout(Some(left)).is_err() {
            break;
        }
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                let room = PROBE_LIMIT - response.len();
                response.extend_from_slice(&buffer[..read.min(room)]);
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    // A deadline can still leave a complete response behind; judge the bytes.
    if response.is_empty() {
        Responder::Silent
    } else {
        responder_from(&response)
    }
}

/// Total time the `service status` probe may take.
const PROBE_BUDGET: std::time::Duration = std::time::Duration::from_secs(3);
/// Bytes of the response kept for judging.
const PROBE_LIMIT: usize = 64 * 1024;

fn responder_from(response: &[u8]) -> Responder {
    let text = String::from_utf8_lossy(response);
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return Responder::Unidentified;
    };
    let ok = head
        .lines()
        .next()
        .is_some_and(|status| status.split_whitespace().nth(1) == Some("200"));
    let chunked = head.lines().any(|line| {
        line.to_ascii_lowercase()
            .starts_with("transfer-encoding: chunked")
    });
    if !ok || chunked {
        return Responder::Unidentified;
    }
    serde_json::from_str::<BackgroundService>(body.trim())
        .ok()
        .and_then(|status| status.responder_pid)
        .filter(|pid| *pid != 0)
        .map_or(Responder::Unidentified, Responder::Process)
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
    asker: Asker,
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
    Ok(status(asker))
}

/// `bruhs-server service install|uninstall|status`. Returns an exit code when handled.
pub fn cli(web_dist: &Path) -> Option<i32> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("service") {
        return None;
    }
    let asker = Asker::Cli {
        port: crate::config::Config::from_env()
            .map_or(crate::config::DEFAULT_PORT, |config| config.port),
    };
    let result = match args.next().as_deref() {
        Some("install") => apply(BackgroundServiceAction::Install, web_dist, asker),
        Some("uninstall") => apply(BackgroundServiceAction::Uninstall, web_dist, asker),
        Some("status") | None => Ok(status(asker)),
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
            if let Some(pid) = status.service_pid {
                println!("Service process: {pid}");
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
    fn only_the_services_own_answering_process_counts_as_serving() {
        use Readiness::{NotInstalled, NotRunning, PortHeld, Serving, Silent, Unverified, Waiting};
        let process = Responder::Process;
        assert_eq!(readiness(false, false, None, process(7)), NotInstalled);
        assert_eq!(readiness(true, false, None, process(7)), Waiting);
        // Loaded without a process, or with pid 0, is not running.
        assert_eq!(readiness(true, true, None, process(7)), NotRunning);
        assert_eq!(readiness(true, true, Some(0), process(0)), NotRunning);
        // Matching, mismatched and zero responders.
        assert_eq!(readiness(true, true, Some(7), process(7)), Serving);
        assert_eq!(readiness(true, true, Some(7), process(8)), PortHeld);
        assert_eq!(readiness(true, true, Some(7), process(0)), PortHeld);
        // Silence and an answer without identity never establish serving.
        assert_eq!(readiness(true, true, Some(7), Responder::Silent), Silent);
        assert_eq!(
            readiness(true, true, Some(7), Responder::Unidentified),
            Unverified
        );
    }

    #[test]
    fn readiness_wording_never_claims_a_stuck_service_is_running() {
        let cli = Asker::Cli { port: 3001 };
        let server = Asker::Server { pid: 1 };
        let log = Some("/logs/bruhs-server.service.log");
        assert_eq!(
            Readiness::Serving.detail(log, cli),
            "Automations keep running after the app quits and start at login."
        );
        assert_eq!(
            Readiness::Silent.detail(log, cli),
            "Installed and started, but it is not answering on 127.0.0.1:3001. Its log is at /logs/bruhs-server.service.log."
        );
        assert!(Readiness::Unverified
            .detail(None, cli)
            .contains("readiness is unverified"));
        assert_eq!(
            Readiness::PortHeld.detail(log, server),
            "Installed. The service takes over once this server exits."
        );
        for state in [
            Readiness::Waiting,
            Readiness::NotRunning,
            Readiness::PortHeld,
            Readiness::Silent,
            Readiness::Unverified,
        ] {
            for asker in [cli, server] {
                assert!(!state.detail(log, asker).contains("keep running"));
            }
        }
    }

    #[test]
    fn the_job_pid_is_read_from_launchctl_and_zero_means_none() {
        let printed = "gui/501/dev.bruhs.server = {\n\tactive count = 1\n\tstate = running\n\tpid = 44689\n\tendpoints = {\n\t\tpid = 1\n\t}\n}";
        assert_eq!(launchd_pid(printed), Some(44689));
        assert_eq!(launchd_pid("\tstate = waiting\n"), None);
        assert_eq!(launchd_pid("\tpid = 0\n"), None);
        assert_eq!(positive_pid("0"), None);
        assert_eq!(positive_pid("x"), None);
    }

    #[test]
    fn a_probe_identifies_only_well_formed_statuses_that_name_their_process() {
        let response = |status: &str, headers: &str, body: &str| {
            format!("HTTP/1.1 {status}\r\ncontent-type: application/json{headers}\r\n\r\n{body}")
                .into_bytes()
        };
        let current = r#"{"supported":true,"installed":true,"loaded":true,"detail":"x","serving":true,"responder_pid":44689}"#;
        assert_eq!(
            responder_from(&response("200 OK", "", current)),
            Responder::Process(44689)
        );
        // An older server's status has no identity: readiness is unverified.
        let old = r#"{"supported":true,"installed":true,"loaded":true,"binary":null,"detail":"x"}"#;
        assert_eq!(
            responder_from(&response("200 OK", "", old)),
            Responder::Unidentified
        );
        let zero =
            r#"{"supported":true,"installed":true,"loaded":true,"detail":"x","responder_pid":0}"#;
        assert_eq!(
            responder_from(&response("200 OK", "", zero)),
            Responder::Unidentified
        );
        for bad in [
            response("404 Not Found", "", current),
            response("200 OK", "\r\ntransfer-encoding: chunked", current),
            response("200 OK", "", "not json"),
            b"HTTP/1.1 200 OK".to_vec(),
        ] {
            assert_eq!(responder_from(&bad), Responder::Unidentified);
        }
    }

    #[test]
    fn older_status_json_reads_with_safe_defaults() {
        let old: BackgroundService = serde_json::from_str(
            r#"{"supported":true,"installed":true,"loaded":true,"binary":"/b","detail":"x"}"#,
        )
        .unwrap();
        assert!(old.loaded);
        assert!(!old.serving, "loaded never implies serving");
        assert_eq!(
            (old.service_pid, old.responder_pid, old.log_path),
            (None, None, None)
        );
    }

    #[test]
    fn a_closed_port_is_silence_not_an_unverified_answer() {
        // Bind and drop to find a port nothing listens on.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| listener.local_addr())
            .map(|address| address.port())
            .unwrap();
        assert_eq!(probe(port), Responder::Silent);
    }

    /// A loopback responder that reads the request, then runs `respond`.
    fn responder(
        respond: impl FnOnce(std::net::TcpStream) + Send + 'static,
    ) -> std::net::SocketAddr {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0_u8; 1024];
                let _ = stream.read(&mut request);
                respond(stream);
            }
        });
        address
    }

    #[test]
    fn a_dripping_responder_cannot_hold_the_probe_past_its_deadline() {
        use std::io::Write;
        use std::time::{Duration, Instant};
        // One byte every 50 ms for 10 s: each read succeeds, so only an
        // overall deadline ends the probe.
        let address = responder(|mut stream| {
            for _ in 0..200 {
                if stream.write_all(b"H").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let started = Instant::now();
        let result = probe_within(address, Duration::from_millis(500));
        let took = started.elapsed();
        assert!(took < Duration::from_secs(2), "probe took {took:?}");
        // Bytes arrived but never a status naming its process.
        assert_eq!(result, Responder::Unidentified);
    }

    #[test]
    fn a_complete_answer_on_a_kept_open_connection_still_identifies_its_process() {
        use std::io::Write;
        use std::time::{Duration, Instant};
        let address = responder(|mut stream| {
            let body = r#"{"supported":true,"installed":true,"loaded":true,"detail":"x","responder_pid":4242}"#;
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            );
            // Ignore Connection: close and keep the socket open.
            std::thread::sleep(Duration::from_secs(5));
        });
        let started = Instant::now();
        let result = probe_within(address, Duration::from_millis(500));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(result, Responder::Process(4242));
    }

    #[test]
    fn a_responder_that_accepts_but_never_writes_is_silence() {
        use std::time::Duration;
        let address = responder(|_stream| std::thread::sleep(Duration::from_secs(5)));
        assert_eq!(
            probe_within(address, Duration::from_millis(300)),
            Responder::Silent
        );
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
