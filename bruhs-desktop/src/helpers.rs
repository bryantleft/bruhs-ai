//! The packaged app (`Bruhs.app`) carries `bruhs-server` (the API and the web
//! app) and `bruhs-capture` (meeting capture) next to the desktop binary, so
//! one download runs everything. On launch the desktop asks each helper's
//! port who is there (`/api/health`, `/health`):
//!
//! - nothing: start the bundled helper;
//! - the same helper from this build: use it;
//! - the same helper from another build (an older app still quitting, or a
//!   `scripts/dev.sh` stack): give it a few seconds to exit, then start ours,
//!   or use it if it stays;
//! - anything else: start nothing and log it, so no data goes to a stranger.
//!
//! Helpers it starts get `BRUHS_EXIT_WITH_PARENT=1` (they exit if the app
//! dies) and are stopped when the app quits. A source build (`cargo run`)
//! has no bundle and starts nothing.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use gpui::App;

/// One bundled helper.
struct Helper {
    binary: &'static str,
    port_env: &'static str,
    default_port: u16,
    health_path: &'static str,
}

const HELPERS: [Helper; 2] = [
    Helper {
        binary: "bruhs-server",
        port_env: "BRUHS_SERVER_PORT",
        default_port: 3001,
        health_path: "/api/health",
    },
    Helper {
        binary: "bruhs-capture",
        port_env: "BRUHS_CAPTURE_PORT",
        default_port: 3002,
        health_path: "/health",
    },
];

/// The commit every binary in this bundle was built from.
const BUILD: &str = env!("BRUHS_BUILD_COMMIT");
/// How long a helper from another build gets to leave the port.
const HANDOVER: Duration = Duration::from_secs(3);
/// How long a helper gets after SIGTERM before it is killed.
const STOP_GRACE: Duration = Duration::from_secs(5);

static STARTED: Mutex<Vec<Child>> = Mutex::new(Vec::new());

fn port(helper: &Helper) -> u16 {
    std::env::var(helper.port_env)
        .ok()
        .and_then(|p| p.trim().parse().ok())
        .unwrap_or(helper.default_port)
}

fn answering(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok()
}

/// Who answers on a helper's port.
#[derive(Debug, PartialEq, Eq)]
enum Occupant {
    Nobody,
    /// The same helper, built from `build`.
    Ours {
        build: String,
    },
    Stranger,
}

fn occupant(helper: &Helper, port: u16) -> Occupant {
    if !answering(port) {
        return Occupant::Nobody;
    }
    probe(port, helper.health_path)
        .and_then(|body| identify(&body, helper.binary))
        .map_or(Occupant::Stranger, |build| Occupant::Ours { build })
}

/// `GET path` on the loopback port; the response body.
fn probe(port: u16, path: &str) -> Option<String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(300)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(1))).ok()?;
    stream
        .set_write_timeout(Some(Duration::from_secs(1)))
        .ok()?;
    let request =
        format!("GET {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).ok()?;
    let mut response = Vec::new();
    stream.take(64 * 1024).read_to_end(&mut response).ok()?;
    let response = String::from_utf8_lossy(&response);
    let (head, body) = response.split_once("\r\n\r\n")?;
    let ok = head.starts_with("HTTP/1.") && head.split_whitespace().nth(1) == Some("200");
    ok.then(|| body.to_string())
}

/// The build of a health body that names `app`.
fn identify(body: &str, app: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    (json.get("app")?.as_str()? == app)
        .then(|| json.get("build")?.as_str().map(str::to_string))
        .flatten()
}

/// The bundle's `Contents/MacOS` and `Contents/Resources`, when running from
/// one.
fn bundle_dirs() -> Option<(PathBuf, PathBuf)> {
    let app = bruhs_core::updates::release::current_app_bundle()?;
    let contents = app.join("Contents");
    Some((contents.join("MacOS"), contents.join("Resources")))
}

fn log_file(name: &str) -> Stdio {
    crate::paths::log_dir()
        .and_then(|dir| {
            std::fs::create_dir_all(&dir).ok()?;
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join(format!("{name}.log")))
                .ok()
        })
        .map_or_else(Stdio::null, Stdio::from)
}

fn spawn(binary: &Path, resources: &Path, helper: &Helper) -> std::io::Result<Child> {
    let mut command = Command::new(binary);
    command
        .env("BRUHS_EXIT_WITH_PARENT", "1")
        .stdin(Stdio::null())
        .stdout(log_file(helper.binary))
        .stderr(log_file(helper.binary));
    if helper.binary == "bruhs-server" {
        command.env("BRUHS_WEB_DIST", resources.join("web"));
    }
    command.spawn()
}

/// Start the bundled helpers that are not already running (off the main
/// thread, since a handover can take seconds), and stop them when the app
/// quits.
pub fn start(cx: &mut App) {
    let Some((macos, resources)) = bundle_dirs() else {
        return;
    };
    for helper in &HELPERS {
        let binary = macos.join(helper.binary);
        if !binary.is_file() {
            continue;
        }
        let resources = resources.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("start-{}", helper.binary))
            .spawn(move || start_one(helper, &binary, &resources));
        if let Err(error) = spawned {
            tracing::error!("could not start {}: {error}", helper.binary);
        }
    }
    cx.on_app_quit(|_cx| async { stop() }).detach();
}

fn start_one(helper: &Helper, binary: &Path, resources: &Path) {
    let port = port(helper);
    let name = helper.binary;
    match occupant(helper, port) {
        Occupant::Nobody => {}
        Occupant::Ours { build } if build == BUILD => {
            tracing::info!("{name}: this build already serves :{port}; using it");
            return;
        }
        Occupant::Ours { build } => {
            tracing::info!("{name}: build {build:?} serves :{port}; waiting for it to exit");
            let deadline = Instant::now() + HANDOVER;
            while answering(port) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(100));
            }
            if answering(port) {
                tracing::warn!("{name}: build {build:?} kept :{port}; using it");
                return;
            }
        }
        Occupant::Stranger => {
            tracing::error!(
                "{name}: something that is not {name} holds :{port}; not starting it. \
                 Free the port or set {} to another one.",
                helper.port_env
            );
            return;
        }
    }
    match spawn(binary, resources, helper) {
        Ok(child) => {
            tracing::info!("started {name} (pid {}) on :{port}", child.id());
            STARTED
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(child);
        }
        Err(error) => tracing::error!("could not start {name}: {error}"),
    }
}

/// Stop the helpers this launch started.
pub fn stop() {
    let mut started = STARTED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for mut child in started.drain(..) {
        terminate(&child);
        // Give it time to shut down cleanly, then make sure.
        let deadline = Instant::now() + STOP_GRACE;
        while Instant::now() < deadline && !matches!(child.try_wait(), Ok(Some(_))) {
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(unix)]
fn terminate(child: &Child) {
    if let Ok(pid) = i32::try_from(child.id()) {
        // SAFETY: the PID of a child this process spawned and has not yet
        // waited on, so it cannot have been reused; no pointers are involved.
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
    }
}

#[cfg(not(unix))]
fn terminate(_child: &Child) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_build_has_no_bundle() {
        // `cargo test` runs from target/, never inside an .app.
        assert!(bundle_dirs().is_none());
    }

    #[test]
    fn only_a_matching_health_body_identifies_a_helper() {
        let body = r#"{"ok":true,"app":"bruhs-server","version":"0.1.0","build":"abc"}"#;
        assert_eq!(identify(body, "bruhs-server").as_deref(), Some("abc"));
        assert_eq!(identify(body, "bruhs-capture"), None);
        assert_eq!(identify(r#"{"ok":true}"#, "bruhs-server"), None);
        assert_eq!(identify("<html>", "bruhs-server"), None);
    }

    /// A one-shot HTTP server on a free port that answers `reply`.
    fn serve_once(reply: &'static str) -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            // `occupant` connects twice: the liveness check, then the probe.
            for stream in listener.incoming().take(2) {
                let mut stream = stream.unwrap();
                // Read the whole request, so closing does not reset it.
                let mut seen = Vec::new();
                let mut buf = [0_u8; 256];
                while !seen.ends_with(b"\r\n\r\n") {
                    match stream.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => seen.extend_from_slice(&buf[..n]),
                    }
                }
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        port
    }

    #[test]
    fn the_port_occupant_is_probed() {
        let ours = serve_once(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"app\":\"bruhs-server\",\"build\":\"b1\"}",
        );
        assert_eq!(
            occupant(&HELPERS[0], ours),
            Occupant::Ours { build: "b1".into() }
        );
        let stranger = serve_once("HTTP/1.1 200 OK\r\n\r\nhello");
        assert_eq!(occupant(&HELPERS[0], stranger), Occupant::Stranger);
        let missing =
            serve_once("HTTP/1.1 404 Not Found\r\n\r\n{\"app\":\"bruhs-server\",\"build\":\"b1\"}");
        assert_eq!(occupant(&HELPERS[0], missing), Occupant::Stranger);
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = free.local_addr().unwrap().port();
        drop(free);
        assert_eq!(occupant(&HELPERS[0], port), Occupant::Nobody);
    }

    #[test]
    fn a_bound_port_answers() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(answering(port));
        drop(listener);
    }
}
