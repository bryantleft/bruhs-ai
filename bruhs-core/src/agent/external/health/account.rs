//! Explicit sign-in for a saved ACP provider on this device.
use bruhs_models::agent_extensions::{valid_environment_name, AcpInstance};
use serde_json::{json, Value};
use std::{io, path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{mpsc, oneshot},
};

use crate::terminal::{TerminalInfo, TerminalManager, TerminalSpec};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Agent,
    Credentials,
    Terminal,
}
#[derive(Clone)]
pub struct Method {
    pub id: String,
    pub name: String,
    pub description: String,
    pub kind: Kind,
    pub variables: Vec<String>,
    pub link: Option<String>,
}
#[derive(Clone)]
pub struct Account {
    pub methods: Vec<Method>,
    pub logout: bool,
    pub ready: bool,
    pub confirmed: bool,
    pub notice: Option<String>,
    pub website: Option<String>,
}
pub enum Action {
    Discover,
    SignIn(String),
    SignOut,
}
pub enum Event {
    Browser {
        url: String,
        response: oneshot::Sender<bool>,
    },
    Terminal(TerminalManager, TerminalInfo),
    Verifying,
}

fn web_url(v: &Value) -> Option<String> {
    let s = v.as_str().filter(|s| s.len() <= 2048)?;
    let url = url::Url::parse(s).ok()?;
    (matches!(url.scheme(), "https" | "http")
        && url.has_host()
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}
fn methods(v: &Value) -> Vec<Method> {
    let mut out = Vec::new();
    for m in v.as_array().into_iter().flatten() {
        let Some(id) = m["id"]
            .as_str()
            .filter(|s| !s.trim().is_empty() && s.len() <= 128 && !s.chars().any(char::is_control))
        else {
            continue;
        };
        if out.iter().any(|o: &Method| o.id == id) {
            continue;
        }
        let kind = match m["type"].as_str() {
            None | Some("agent") => Kind::Agent,
            Some("terminal") => Kind::Terminal,
            Some("env_var") => Kind::Credentials,
            _ => continue,
        };
        let text = |s: &str, n| {
            s.chars()
                .filter(|c| !c.is_control())
                .take(n)
                .collect::<String>()
        };
        out.push(Method {
            id: id.into(),
            name: text(
                m["name"].as_str().filter(|s| !s.is_empty()).unwrap_or(id),
                128,
            ),
            description: text(m["description"].as_str().unwrap_or_default(), 512),
            kind,
            variables: m["vars"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| {
                    v["name"]
                        .as_str()
                        .filter(|s| s.len() <= 128 && valid_environment_name(s))
                        .map(str::to_owned)
                })
                .take(16)
                .collect(),
            link: web_url(&m["link"]),
        });
        if out.len() == 32 {
            break;
        }
    }
    out
}

async fn command(instance: &AcpInstance, cwd: &Path) -> io::Result<Command> {
    let mut cmd = super::process_command(Path::new(instance.executable()), cwd);
    let managed = crate::secret_sources::for_target(&format!("acp/{}", instance.id)).await?;
    crate::agent_extensions::environment::apply(&mut cmd, instance, &managed).await?;
    cmd.args(&instance.args);
    Ok(cmd)
}

struct Connection {
    #[cfg(unix)]
    _group: super::process::ProcessGroup,
    _child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    initialized: Value,
    credential_variables: Vec<String>,
    next_id: u64,
}
impl Connection {
    async fn start(instance: &AcpInstance, cwd: &Path) -> io::Result<Self> {
        let mut cmd = command(instance, cwd).await?;
        let mut child = cmd.stdin(Stdio::piped()).stderr(Stdio::null()).spawn()?;
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        let mut conn = Self {
            #[cfg(unix)]
            _group: super::process::ProcessGroup::of(&child),
            _child: child,
            input,
            output,
            initialized: Value::Null,
            credential_variables: Vec::new(),
            next_id: 1,
        };
        let params = json!({
            "protocolVersion": 1,
            "clientInfo": {"name": "bruhs-provider-sign-in", "version": env!("CARGO_PKG_VERSION")},
            "clientCapabilities": {"auth": {"terminal": true}, "elicitation": {"url": {}}}
        });
        let result = conn.request("initialize", params, None).await?;
        if result["protocolVersion"] != 1 {
            return Err(io::Error::other("This provider did not negotiate ACP v1."));
        }
        // An anonymous session can succeed even when a credential method has no key.
        for method in methods(&result["authMethods"]) {
            for name in method.variables {
                let configured = cmd
                    .as_std()
                    .get_envs()
                    .find(|(key, _)| {
                        key.to_str().is_some_and(|key| {
                            bruhs_models::agent_extensions::same_environment_name(key, &name)
                        })
                    })
                    .map_or_else(
                        || std::env::var_os(&name).is_some_and(|v| !v.is_empty()),
                        |(_, v)| v.is_some_and(|v| !v.is_empty()),
                    );
                if configured {
                    conn.credential_variables.push(name);
                }
            }
        }
        conn.initialized = result;
        Ok(conn)
    }
    async fn write(&mut self, v: Value) -> io::Result<()> {
        let mut bytes = serde_json::to_vec(&v)?;
        bytes.push(b'\n');
        self.input.write_all(&bytes).await
    }
    async fn request(
        &mut self,
        method: &str,
        params: Value,
        events: Option<&mpsc::Sender<Event>>,
    ) -> io::Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let exchange = async {
            self.write(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
                .await?;
            let mut line = Vec::new();
            loop {
                line.clear();
                let size = (&mut self.output)
                    .take(256 * 1024 + 1)
                    .read_until(b'\n', &mut line)
                    .await?;
                if size == 0 || size > 256 * 1024 {
                    return Err(io::Error::other(
                        "The sign-in process closed or returned an oversized response.",
                    ));
                }
                let msg: Value = serde_json::from_slice(&line)?;
                if msg["jsonrpc"] != "2.0" {
                    return Err(io::Error::other("Invalid ACP response."));
                }
                if msg.get("method").is_none() && msg["id"] == id {
                    if let Some(error) = msg.get("error") {
                        let kind = if error["code"] == -32000 {
                            io::ErrorKind::PermissionDenied
                        } else {
                            io::ErrorKind::Other
                        };
                        return Err(io::Error::new(
                            kind,
                            format!("The agent rejected {method} (ACP {}).", error["code"]),
                        ));
                    }
                    return msg.get("result").cloned().ok_or_else(|| {
                        io::Error::other("The ACP response is missing its result.")
                    });
                }
                let Some(request_id) = msg.get("id").filter(|_| msg.get("method").is_some()) else {
                    continue;
                };
                let response = if matches!(
                    msg["method"].as_str(),
                    Some("elicitation/create" | "session/elicitation" | "_session/elicitation")
                ) {
                    let params = &msg["params"];
                    let mut accepted = false;
                    if params["mode"] == "url"
                        && params["elicitationId"]
                            .as_str()
                            .is_some_and(|s| !s.is_empty() && s.len() <= 128)
                    {
                        if let (Some(events), Some(url)) = (events, web_url(&params["url"])) {
                            let (response, answer) = oneshot::channel();
                            events
                                .send(Event::Browser { url, response })
                                .await
                                .map_err(|_| io::Error::other("Sign-in closed."))?;
                            accepted = answer.await.unwrap_or(false);
                        }
                    }
                    json!({"jsonrpc":"2.0","id":request_id,"result":{"action":if accepted{"accept"}else{"decline"}}})
                } else {
                    json!({"jsonrpc":"2.0","id":request_id,"error":{"code":-32601,"message":"Unsupported during sign-in."}})
                };
                self.write(response).await?;
            }
        };
        if method == "authenticate" {
            return exchange.await;
        }
        tokio::time::timeout(Duration::from_secs(30), exchange)
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("The provider timed out during {method}."),
                )
            })?
    }
    async fn verify(&mut self, cwd: &Path) -> io::Result<()> {
        let session = self
            .request("session/new", json!({"cwd":cwd,"mcpServers":[]}), None)
            .await?;
        if !session["sessionId"].as_str().is_some_and(|s| !s.is_empty()) {
            return Err(io::Error::other(
                "The provider did not return a session after sign-in.",
            ));
        }
        Ok(())
    }
}

pub(super) async fn kiro_agents(instance: &AcpInstance, cwd: &Path) -> io::Result<crate::agent_extensions::kiro::Agents> {
    let mut conn = Connection::start(instance, cwd).await?;
    let template = conn.request("_kiro/config/template", json!({}), None).await?;
    Ok(crate::agent_extensions::kiro::Agents::from_template(&template))
}

struct ActiveTerminal(TerminalManager, String);
impl Drop for ActiveTerminal {
    fn drop(&mut self) {
        self.0.close(&self.1);
    }
}
async fn terminal(
    instance: &AcpInstance,
    cwd: &Path,
    method: &Value,
    events: &mpsc::Sender<Event>,
) -> io::Result<()> {
    let cmd = command(instance, cwd).await?;
    let process = cmd.as_std();
    let mut launch = portable_pty::CommandBuilder::new(process.get_program());
    launch.args(process.get_args());
    for (k, v) in process.get_envs() {
        if let Some(v) = v {
            launch.env(k, v)
        } else {
            launch.env_remove(k)
        }
    }
    if let Some(args) = method.get("args") {
        for arg in args
            .as_array()
            .ok_or_else(|| io::Error::other("Invalid terminal arguments."))?
        {
            launch.arg(
                arg.as_str()
                    .ok_or_else(|| io::Error::other("Invalid terminal argument."))?,
            );
        }
    }
    if let Some(env) = method.get("env") {
        for (name, value) in env
            .as_object()
            .ok_or_else(|| io::Error::other("Invalid terminal environment."))?
        {
            if !valid_environment_name(name) {
                return Err(io::Error::other(
                    "Invalid variable name in terminal method.",
                ));
            }
            if !crate::connections::PRIVATE_ENV
                .iter()
                .any(|key| name.eq_ignore_ascii_case(key))
            {
                launch.env(
                    name,
                    value
                        .as_str()
                        .ok_or_else(|| io::Error::other("Invalid terminal variable."))?,
                );
            }
        }
    }
    let term = TerminalManager::new();
    let manager = term.clone();
    let spec = TerminalSpec {
        cwd: Some(cwd.into()),
        title: Some(format!("Sign in to {}", instance.name)),
        cols: Some(80),
        rows: Some(12),
        ..Default::default()
    };
    let (done, exit) = oneshot::channel();
    // A cancelled spawn still closes the PTY when the background result is dropped.
    let (info, _guard) = tokio::task::spawn_blocking(move || {
        let info = manager
            .create_command(spec, Some(launch), Some(done))
            .map_err(io::Error::other)?;
        let guard = ActiveTerminal(manager, info.id.clone());
        Ok::<_, io::Error>((info, guard))
    })
    .await
    .map_err(io::Error::other)??;
    events
        .send(Event::Terminal(term, info))
        .await
        .map_err(|_| io::Error::other("Sign-in closed."))?;
    if !exit.await.unwrap_or(false) {
        return Err(io::Error::other(
            "The provider login command did not finish successfully.",
        ));
    }
    Ok(())
}

// A successful session alone can also mean that the provider allows anonymous access.
struct Confirmation {
    path: std::path::PathBuf,
    binding: String,
}
impl Confirmation {
    fn new(instance: &AcpInstance) -> io::Result<Self> {
        use sha2::{Digest, Sha256};
        let id = Sha256::digest(instance.id.as_bytes());
        let profiles = [
            "HOME",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_STATE_HOME",
            "APPDATA",
            "LOCALAPPDATA",
        ]
        .map(std::env::var_os);
        let secrets = crate::secret_sources::load()?;
        let target = format!("acp/{}", instance.id);
        let bindings: Vec<_> = secrets
            .bindings
            .iter()
            .filter(|b| b.target == target)
            .map(|b| (b, secrets.sources.get(&b.source)))
            .collect();
        let binding = Sha256::digest(serde_json::to_vec(&(
            instance.executable(),
            &instance.args,
            &instance.environment,
            &instance.registry,
            bindings,
            profiles,
        ))?);
        let path = crate::agent_extensions::directory()?
            .join("acp-auth")
            .join(format!("{id:x}"));
        Ok(Self {
            path,
            binding: format!("{binding:x}"),
        })
    }
    fn lock(&self) -> io::Result<std::fs::File> {
        crate::agent_extensions::private_directory(self.path.parent().expect("auth directory"))?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.path.with_extension("lock"))?;
        file.try_lock().map_err(|e| match e {
            std::fs::TryLockError::WouldBlock => {
                io::Error::other("Another sign-in operation is running for this provider.")
            }
            std::fs::TryLockError::Error(e) => e,
        })?;
        Ok(file)
    }
    fn confirmed(&self) -> bool {
        std::fs::read_to_string(&self.path).is_ok_and(|v| v == self.binding)
    }
    fn set(&self, value: bool) -> io::Result<()> {
        if !value {
            if self.confirmed() {
                std::fs::remove_file(&self.path)?;
            }
            return Ok(());
        }
        use std::io::Write;
        let dir = self.path.parent().expect("auth directory");
        crate::agent_extensions::private_directory(dir)?;
        let mut file = tempfile::NamedTempFile::new_in(dir)?;
        file.write_all(self.binding.as_bytes())?;
        file.persist(&self.path)?;
        Ok(())
    }
}
pub async fn run(id: &str, action: Action, events: mpsc::Sender<Event>) -> io::Result<Account> {
    let instance = crate::agent_extensions::load_config()?
        .acp
        .into_iter()
        .find(|i| i.id == id)
        .ok_or_else(|| io::Error::other("This provider was removed."))?;
    if instance.driver.is_some() || !instance.enabled {
        return Err(io::Error::other(
            "Enable this ACP provider before signing in.",
        ));
    }
    let dir = tempfile::tempdir()?;
    let cwd = dir.path();
    let marker = Confirmation::new(&instance)?;
    let _lock = marker.lock()?;
    let mut conn = Connection::start(&instance, cwd).await?;
    let advertised = methods(&conn.initialized["authMethods"]);
    let logout = !conn.initialized["agentCapabilities"]["auth"]["logout"].is_null()
        && conn.initialized["agentCapabilities"]["auth"]["logout"] != false;
    let mut notice = None;
    let explicit = matches!(action, Action::SignIn(_));
    let ready = match action {
        Action::Discover => match conn.verify(cwd).await {
            Ok(()) => true,
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => false,
            Err(e) => {
                notice = Some(e.to_string());
                false
            }
        },
        Action::SignOut => {
            if !logout {
                return Err(io::Error::other("This agent does not offer sign-out."));
            }
            conn.request("logout", json!({}), None).await?;
            false
        }
        Action::SignIn(id) => {
            let method = advertised.iter().find(|m| m.id == id).ok_or_else(|| {
                io::Error::other("The agent no longer advertises this sign-in method.")
            })?;
            match method.kind {
                Kind::Terminal => {
                    let raw = conn.initialized["authMethods"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|m| m["id"] == id)
                        .unwrap()
                        .clone();
                    drop(conn);
                    terminal(&instance, cwd, &raw, &events).await?;
                    conn = Connection::start(&instance, cwd).await?;
                }
                Kind::Agent => {
                    conn.request("authenticate", json!({"methodId":id}), Some(&events))
                        .await?;
                }
                Kind::Credentials => {
                    if !method.variables.is_empty()
                        && !method
                            .variables
                            .iter()
                            .any(|name| conn.credential_variables.contains(name))
                    {
                        return Err(io::Error::other(
                            "Configure the provider’s credential variables first.",
                        ));
                    }
                }
            }
            events
                .send(Event::Verifying)
                .await
                .map_err(|_| io::Error::other("Sign-in closed."))?;
            conn.verify(cwd).await?;
            true
        }
    };
    if explicit || (!ready && notice.is_none()) {
        marker.set(ready)?;
    }
    let website = if advertised.is_empty() {
        instance
            .registry
            .as_ref()
            .and_then(|r| crate::agent_extensions::registry::cached_provider_website(&r.agent_id))
    } else {
        None
    };
    Ok(Account {
        website,
        confirmed: ready && marker.confirmed(),
        notice,
        methods: advertised,
        logout,
        ready,
    })
}
