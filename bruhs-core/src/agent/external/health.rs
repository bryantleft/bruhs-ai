//! Read-only local provider checks used by Settings and its background timer.
pub mod account;
use super::{executable, process, AgentProvider};
use futures_util::{stream, StreamExt};
use serde_json::{json, Value};
use std::{io, path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::Command,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    SignedIn,
    SignedOut,
    Configured,
    Installed,
    Ready,
    NotInstalled,
    Disabled,
    TimedOut,
    Failed,
}
impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::SignedIn => "Signed in",
            Self::SignedOut => "Signed out",
            Self::Configured => "Credentials configured",
            Self::Ready => "Protocol ready",
            Self::Installed => "Installed",
            Self::NotInstalled => "Not installed",
            Self::Disabled => "Disabled",
            Self::TimedOut => "Check timed out",
            Self::Failed => "Check failed",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ProviderHealth {
    pub id: String,
    pub name: String,
    pub provider: AgentProvider,
    pub status: Status,
    pub environment_enabled: bool,
    pub accent_color: Option<u32>,
    pub version: Option<String>,
    pub model_count: Option<usize>,
}
impl ProviderHealth {
    pub fn new(provider: AgentProvider, status: Status) -> Self {
        Self {
            id: provider.as_str().into(),
            name: provider.label().into(),
            provider,
            status,
            environment_enabled: true,
            version: None,
            accent_color: None,
            model_count: None,
        }
    }
}

// Native adapters and configured ACP agents share the same executable selection as a real turn.
pub async fn external_provider_health(
    prefs: &crate::agent_extensions::availability::ProviderAvailability,
) -> Vec<ProviderHealth> {
    use AgentProvider::*;
    let builtin = stream::iter([Cursor, Antigravity, Grok, OpenCode])
        .map(|provider| async move {
            if !prefs.enabled(provider) {
                return ProviderHealth::new(provider, Status::Disabled);
            }
            let mut row = ProviderHealth::new(provider, Status::Failed);
            if let Ok(program) = executable(provider) {
                row.status = bounded(probe_cli(&program, &mut row)).await;
            }
            row
        })
        .buffered(4)
        .collect::<Vec<_>>();
    let configured = async {
        let Ok(Ok(config)) =
            tokio::task::spawn_blocking(crate::agent_extensions::load_config).await
        else {
            return vec![ProviderHealth::new(Acp, Status::Failed)];
        };
        stream::iter(config.acp)
            .map(|instance| async move {
                let mut row = ProviderHealth::new(Acp, Status::Disabled);
                row.id = format!("acp/{}", instance.id);
                row.environment_enabled = instance.enabled;
                row.name = instance.name.clone();
                row.accent_color = instance
                    .accent_color
                    .as_deref()
                    .and_then(bruhs_models::agent_extensions::parse_accent_color);
                if prefs.acp_enabled(&instance) {
                    row.status = if instance.driver
                        == Some(bruhs_models::agent_extensions::AcpDriver::Kiro)
                    {
                        bounded(probe_kiro(&instance, &mut row)).await
                    } else {
                        bounded(probe_acp(
                            Path::new(instance.executable()),
                            &instance.args,
                            Some(&instance),
                            &mut row,
                        ))
                        .await
                    };
                }
                row
            })
            .buffered(4)
            .collect::<Vec<_>>()
            .await
    };
    let (mut rows, instances) = tokio::join!(builtin, configured);
    rows.extend(instances);
    rows
}

async fn bounded(check: impl std::future::Future<Output = io::Result<Status>>) -> Status {
    match tokio::time::timeout(Duration::from_secs(10), check).await {
        Ok(Ok(status)) => status,
        Ok(Err(e)) if e.kind() == io::ErrorKind::NotFound => Status::NotInstalled,
        Ok(Err(_)) => Status::Failed,
        Err(_) => Status::TimedOut,
    }
}

fn process_command(program: &Path, cwd: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    crate::connections::protect_command(&mut cmd);
    #[cfg(unix)]
    cmd.process_group(0);
    cmd
}

fn command(program: &Path, cwd: &Path) -> Command {
    let mut cmd = process_command(program, cwd);
    cmd.env("NO_COLOR", "1").env("CI", "1");
    cmd
}
const MAX_BYTES: u64 = 256 * 1024;
async fn read_output(r: impl tokio::io::AsyncRead + Unpin) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    if r.take(MAX_BYTES + 1).read_to_end(&mut out).await? as u64 > MAX_BYTES {
        return Err(io::Error::other(
            "Provider status exceeded its output limit",
        ));
    }
    Ok(out)
}
async fn run(program: &Path, args: &[&str], cwd: &Path) -> io::Result<(bool, String)> {
    let (success, stdout, stderr) = output(command(program, cwd).args(args)).await?;
    Ok((success, format!("{stdout}\n{stderr}")))
}
async fn output(cmd: &mut Command) -> io::Result<(bool, String, String)> {
    let mut child = cmd.spawn()?;
    #[cfg(unix)]
    let mut group = process::ProcessGroup::of(&child);
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("child stdout was not piped"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("child stderr was not piped"))?;
    let (stdout, stderr) = tokio::try_join!(read_output(stdout), read_output(stderr))?;
    let success = child.wait().await?.success();
    #[cfg(unix)]
    group.release();
    Ok((
        success,
        bruhs_models::runtime_models::strip_ansi(&String::from_utf8_lossy(&stdout)),
        bruhs_models::runtime_models::strip_ansi(&String::from_utf8_lossy(&stderr)),
    ))
}

async fn probe_kiro(
    instance: &bruhs_models::agent_extensions::AcpInstance,
    row: &mut ProviderHealth,
) -> io::Result<Status> {
    use crate::agent_extensions::{environment, kiro::Catalog};
    let directory = tempfile::tempdir()?;
    let managed = crate::secret_sources::for_target(&format!("acp/{}", instance.id)).await?;
    let invoke = async |args: &[&str]| -> io::Result<(bool, String, String)> {
        let mut cmd = command(Path::new(&instance.command), directory.path());
        environment::apply(&mut cmd, instance, &managed).await?;
        output(cmd.args(args)).await
    };
    let (ok, stdout, stderr) = invoke(&["--version"]).await?;
    if !ok {
        return Ok(Status::Failed);
    }
    row.version = version(&format!("{stdout}\n{stderr}"));
    let (ok, account, _) = invoke(&["whoami", "--format", "json"]).await?;
    if !ok {
        Catalog::default().save(instance)?;
        return Ok(
            if account
                .trim()
                .lines()
                .next()
                .and_then(|line| serde_json::from_str::<Value>(line).ok())
                .is_some_and(|value| value.get("account").is_some_and(Value::is_null))
            {
                Status::SignedOut
            } else {
                Status::Failed
            },
        );
    }
    let (models, agents) = tokio::join!(
        invoke(&["chat", "--list-models", "--format", "json"]),
        tokio::time::timeout(
            Duration::from_secs(2),
            account::kiro_agents(instance, directory.path())
        ),
    );
    let mut catalog = match models {
        Ok((true, text, _)) => Catalog::parse(&text, instance).unwrap_or_default(),
        _ => Catalog::default(),
    };
    catalog.agents = agents.ok().and_then(Result::ok).unwrap_or_default();
    row.model_count = Some(catalog.choices(instance).len());
    catalog.save(instance)?;
    Ok(Status::SignedIn)
}

fn version(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .map(|t| t.trim_start_matches('v'))
        .find(|s| {
            s.len() <= 72
                && s.starts_with(|c: char| c.is_ascii_digit())
                && s.contains('.')
                && s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b".-+".contains(&c))
        })
        .map(str::to_owned)
}
pub async fn native_cli_version(program: &Path) -> Result<Option<String>, Status> {
    let check = async {
        let cwd = tempfile::tempdir()?;
        let (ok, text) = run(program, &["--version"], cwd.path()).await?;
        if !ok {
            return Err(io::Error::other("Version check failed"));
        }
        Ok(version(&text))
    };
    match tokio::time::timeout(Duration::from_secs(5), check).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) if error.kind() == io::ErrorKind::NotFound => Err(Status::NotInstalled),
        Ok(Err(_)) => Err(Status::Failed),
        Err(_) => Err(Status::TimedOut),
    }
}

fn sign_in_status(success: bool, text: &str) -> Status {
    let s = text.to_ascii_lowercase();
    if [
        "not logged in",
        "unauthenticated",
        "not authenticated",
        "not signed in",
        "signed out",
        "please log in",
        "please sign in",
    ]
    .iter()
    .any(|v| s.contains(v))
    {
        Status::SignedOut
    } else if !success {
        Status::Failed
    } else if ["logged in", "signed in", "authenticated"]
        .iter()
        .any(|v| s.contains(v))
    {
        Status::SignedIn
    } else {
        Status::Installed
    }
}

async fn probe_cli(program: &Path, row: &mut ProviderHealth) -> io::Result<Status> {
    let cwd = tempfile::tempdir()?;
    let (ok, output) = run(program, &["--version"], cwd.path()).await?;
    if !ok {
        return Ok(Status::Failed);
    }
    row.version = version(&output);
    match row.provider {
        AgentProvider::Cursor => {
            let (ok, text) = run(program, &["status"], cwd.path()).await?;
            let status = sign_in_status(ok, &text);
            if status == Status::SignedIn {
                if let Ok(models) =
                    tokio::time::timeout(Duration::from_secs(3), super::models::refresh_cursor())
                        .await
                {
                    if models.error.is_none() {
                        row.model_count = Some(models.models.len());
                    }
                }
            }
            Ok(status)
        }
        AgentProvider::Grok => {
            let (ok, text) = run(program, &["models"], cwd.path()).await?;
            if ok {
                let models = text
                    .lines()
                    .filter_map(|l| {
                        l.trim()
                            .strip_prefix("- ")
                            .or_else(|| l.trim().strip_prefix("* "))
                    })
                    .filter_map(|l| l.split_whitespace().next())
                    .collect::<std::collections::BTreeSet<_>>();
                if !models.is_empty() {
                    row.model_count = Some(models.len());
                }
            }
            Ok(sign_in_status(ok, &text))
        }
        AgentProvider::Antigravity => {
            let status = probe_acp(program, &["acp".into()], None, row).await?;
            if status == Status::Ready {
                if let Ok(models) = tokio::time::timeout(
                    Duration::from_secs(4),
                    super::models::refresh_antigravity(),
                )
                .await
                {
                    if models.error.is_none() && !models.models.is_empty() {
                        row.model_count = Some(models.models.len());
                    }
                }
            }
            Ok(status)
        }
        AgentProvider::OpenCode => {
            let (ok, text) = run(program, &["auth", "list", "--pure"], cwd.path()).await?;
            if !ok {
                return Ok(Status::Failed);
            }
            let tokens: Vec<_> = text.split_whitespace().collect();
            let configured = tokens.windows(2).any(|pair| {
                pair[0].parse::<u32>().is_ok_and(|n| n > 0)
                    && (pair[1].starts_with("credential") || pair[1] == "environment")
            });
            if let Ok(Ok((true, models))) = tokio::time::timeout(
                Duration::from_secs(3),
                run(program, &["models", "--pure"], cwd.path()),
            )
            .await
            {
                row.model_count = Some(
                    models
                        .lines()
                        .map(str::trim)
                        .filter(|s| s.contains('/') && !s.chars().any(char::is_whitespace))
                        .count(),
                );
            }
            Ok(if configured {
                Status::Configured
            } else {
                Status::Installed
            })
        }
        _ => Ok(Status::Installed),
    }
}

// Initialization establishes that the configured command speaks ACP. It does
// not authenticate, create a session, open a project, or submit a prompt.
async fn probe_acp(
    program: &Path,
    args: &[String],
    instance: Option<&bruhs_models::agent_extensions::AcpInstance>,
    row: &mut ProviderHealth,
) -> io::Result<Status> {
    let cwd = tempfile::tempdir()?;
    let mut cmd = command(program, cwd.path());
    if let Some(instance) = instance {
        let managed = crate::secret_sources::for_target(&format!("acp/{}", instance.id)).await?;
        crate::agent_extensions::environment::apply(&mut cmd, instance, &managed).await?;
    }
    let mut child = cmd
        .args(args)
        .stdin(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    #[cfg(unix)]
    let _group = process::ProcessGroup::of(&child);
    let request = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientInfo":{"name":"bruhs","version":env!("CARGO_PKG_VERSION")},"clientCapabilities":{}}});
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("child stdin was not piped"))?;
    stdin.write_all(format!("{request}\n").as_bytes()).await?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("child stdout was not piped"))?;
    let mut lines = BufReader::new(stdout.take(MAX_BYTES + 1)).lines();
    while let Some(line) = lines.next_line().await? {
        let Ok(frame) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if frame["id"] != 1 || frame.get("method").is_some() {
            continue;
        }
        if frame["jsonrpc"] != "2.0" || frame["result"]["protocolVersion"] != 1 {
            return Ok(Status::Failed);
        }
        if let Some(version) = frame["result"]["agentInfo"]["version"]
            .as_str()
            .and_then(version)
        {
            row.version = Some(version);
        }
        return Ok(Status::Ready);
    }
    Ok(Status::Failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_only_observed_authentication() {
        assert_eq!(sign_in_status(true, "unauthenticated"), Status::SignedOut);
        assert_eq!(sign_in_status(true, "Not logged in"), Status::SignedOut);
        assert_eq!(sign_in_status(true, "You are logged in"), Status::SignedIn);
        assert_eq!(sign_in_status(true, "CLI running"), Status::Installed);
        assert_eq!(sign_in_status(false, "Service unavailable"), Status::Failed);
    }

    #[tokio::test(start_paused = true)]
    async fn distinguishes_missing_executable_and_timeout() {
        let missing = async { Err(io::Error::from(io::ErrorKind::NotFound)) };
        assert_eq!(bounded(missing).await, Status::NotInstalled);
        assert_eq!(bounded(std::future::pending()).await, Status::TimedOut);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn agent_must_answer_initialize_with_a_supported_protocol() {
        let mut row = ProviderHealth::new(AgentProvider::Acp, Status::Failed);
        for (protocol, expected) in [(2, Status::Failed), (1, Status::Ready)] {
            let frame = json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":protocol,"agentInfo":{"version":"v3.7.1"}}});
            let script = format!("read -r request || exit 1\nprintf '%s\\n' '{frame}'");
            let args = ["-c".to_owned(), script];
            assert_eq!(
                bounded(probe_acp(Path::new("/bin/sh"), &args, None, &mut row)).await,
                expected
            );
        }
        assert_eq!(row.version.as_deref(), Some("3.7.1"));
    }
}
