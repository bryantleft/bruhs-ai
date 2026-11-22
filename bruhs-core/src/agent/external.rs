//! Headless agent adapters. Each CLI keeps its own login and model configuration.

use super::{AgentProvider, McpServerInfo, PermissionMode, QueryEvent, TurnContext, UsageInfo};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

pub(super) mod acp;
pub mod health;
pub(crate) mod metadata;
pub(super) mod models;
pub(super) mod writer_catalog;
#[path = "../../../claude-sdk-rs/src/process.rs"]
pub(super) mod process;

#[derive(Clone)]
pub(crate) struct Options {
    provider: AgentProvider,
    pub(super) kiro_selection: crate::agent_extensions::kiro::Selection,
    pub(super) secret_env: Vec<(String, String)>,
    prompt: String,
    cwd: PathBuf,
    model: String,
    mode: PermissionMode,
    context: Option<String>,
    resume: Option<String>,
    instructions: String,
    mcp: HashMap<String, Mcp>,
    unattended: bool,
    agent_defaults: Value,
    permission_defaults: Vec<Value>,
}

#[derive(Clone)]
struct Mcp {
    command: String,
    args: Vec<String>,
    env: HashMap<String, String>,
}

impl Options {
    pub(super) fn new(
        provider: AgentProvider,
        prompt: &str,
        cwd: &str,
        model: &str,
        mode: PermissionMode,
        context: Option<&str>,
        resume: Option<&str>,
    ) -> Self {
        Self {
            provider,
            secret_env: Vec::new(),
            kiro_selection: Default::default(),
            prompt: prompt.into(),
            cwd: cwd.into(),
            model: model.into(),
            mode,
            context: context.map(String::from),
            resume: resume.map(String::from),
            instructions: String::new(),
            mcp: HashMap::new(),
            unattended: false,
            agent_defaults: json!({}),
            permission_defaults: Vec::new(),
        }
    }

    pub(crate) fn append_instructions(&mut self, instructions: &str) {
        if !self.instructions.is_empty() {
            self.instructions.push_str("\n\n");
        }
        self.instructions.push_str(instructions);
    }

    pub(super) fn unattended(&mut self) {
        self.unattended = true;
    }

    pub(super) fn supports_mcp(&self) -> bool {
        matches!(self.provider, AgentProvider::OpenCode | AgentProvider::Acp)
    }

    pub(crate) fn add_mcp(
        &mut self,
        name: &str,
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> io::Result<()> {
        if !self.supports_mcp() {
            return Err(io::Error::other(format!(
                "{} does not expose a per-turn MCP configuration override.",
                self.provider.label()
            )));
        }
        self.mcp.insert(
            name.into(),
            Mcp {
                command: command.into(),
                args: args.into(),
                env: env.clone(),
            },
        );
        Ok(())
    }

    fn prompt(&self) -> String {
        let mut prompt = String::new();
        if let Some(context) = self.context.as_ref().filter(|_| self.resume.is_none()) {
            prompt.push_str("<thread_context>\n");
            prompt.push_str(context);
            prompt.push_str("\n</thread_context>\n\n");
        }
        if !self.instructions.is_empty() {
            prompt.push_str("<bruhs_instructions>\n");
            prompt.push_str(&self.instructions);
            prompt.push_str("\n</bruhs_instructions>\n\n");
        }
        prompt.push_str(&self.prompt);
        prompt
    }

    fn prepare(&self) -> io::Result<Prepared> {
        if self.unattended {
            return Err(io::Error::other(format!(
                "{} event runs are unavailable because this adapter cannot isolate inherited hooks and plugins. Choose Claude Code or Codex for unattended agents.",
                self.provider.label()
            )));
        }
        if self.provider == AgentProvider::Antigravity
            && self.mode != PermissionMode::BypassPermissions
        {
            return Err(io::Error::other(
                "Antigravity's plan mode is an instruction, not an enforced read-only boundary. This adapter currently requires Full access. Choose another provider for Default, Plan, or Accept edits.",
            ));
        }
        if self
            .resume
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.starts_with('-'))
        {
            return Err(io::Error::other(
                "The saved provider session ID is invalid. Start a fresh session.",
            ));
        }
        let files = tempfile::tempdir()?;
        let mut command = Command::new(executable(self.provider)?);
        command.current_dir(&self.cwd).stdin(Stdio::piped());
        process::apply_env(
            &mut command,
            true,
            &crate::connections::PRIVATE_ENV.map(String::from),
            &self.secret_env,
        );
        let prompt = self.prompt();
        let mut input = prompt.as_bytes().to_vec();
        let model = self
            .model
            .strip_prefix(&format!("{}/", self.provider.as_str()))
            .unwrap_or(&self.model);
        match self.provider {
            AgentProvider::Cursor => {
                cursor_config(self, files.path(), &mut command)?;
                // Older CLIs emit indistinguishable deltas and duplicate flushes
                // in partial mode. Message-level JSON is stable across versions.
                command.args(["--print", "--output-format=stream-json"]);
                if self.mode == PermissionMode::BypassPermissions {
                    command.arg("--force");
                }
                if let Some(resume) = &self.resume {
                    command.arg(format!("--resume={resume}"));
                }
            }
            AgentProvider::Antigravity => {
                command.args(["--input-format=stream-json", "--output-format=stream-json"]);
                match self.mode {
                    PermissionMode::Default | PermissionMode::Plan => {
                        command.arg("--mode=plan");
                    }
                    PermissionMode::AcceptEdits => {
                        command.arg("--mode=accept-edits");
                    }
                    PermissionMode::BypassPermissions => {
                        command.arg("--dangerously-skip-permissions");
                    }
                }
                if let Some(resume) = &self.resume {
                    command.arg(format!("--conversation={resume}"));
                }
                input = serde_json::to_vec(&json!({"event":"user", "message":{"content":prompt}}))?;
                input.push(b'\n');
            }
            AgentProvider::Grok => {
                let file = files.path().join("prompt.txt");
                std::fs::write(&file, prompt)?;
                command.arg("--prompt-file").arg(file);
                command.args([
                    "--output-format=streaming-messages-json",
                    "--include-partial-messages",
                ]);
                let mode = match self.mode {
                    PermissionMode::Default | PermissionMode::Plan => "plan",
                    PermissionMode::AcceptEdits => "acceptEdits",
                    PermissionMode::BypassPermissions => "bypassPermissions",
                };
                command.arg(format!("--permission-mode={mode}"));
                match self.mode {
                    PermissionMode::Default | PermissionMode::Plan => {
                        command.args([
                            "--tools=read_file,grep,list_dir",
                            "--deny=Bash",
                            "--deny=Edit",
                            "--deny=Write",
                            "--deny=MCPTool",
                            "--no-subagents",
                        ]);
                    }
                    PermissionMode::AcceptEdits => {
                        command.args([
                            "--tools=read_file,grep,list_dir,write,search_replace",
                            "--deny=Bash",
                            "--deny=MCPTool",
                            "--no-subagents",
                        ]);
                    }
                    PermissionMode::BypassPermissions => {}
                }
                if let Some(resume) = &self.resume {
                    command.arg(format!("--resume={resume}"));
                }
                input.clear();
            }
            AgentProvider::OpenCode => {
                let agent = opencode_config(self, &mut command)?;
                command.args(["run", "--format=json", "--thinking"]);
                command.arg(format!("--agent={agent}"));
                if let Some(resume) = &self.resume {
                    command.arg(format!("--session={resume}"));
                }
            }
            AgentProvider::Claude | AgentProvider::Codex | AgentProvider::Acp => {
                return Err(io::Error::other(
                    "This provider uses its dedicated SDK adapter.",
                ));
            }
        }
        if model != "default" && !model.is_empty() {
            command.arg(format!("--model={model}"));
        }
        Ok(Prepared {
            command,
            input,
            _files: files,
        })
    }
}

struct Prepared {
    command: Command,
    input: Vec<u8>,
    _files: tempfile::TempDir,
}

pub(super) fn executable(provider: AgentProvider) -> io::Result<PathBuf> {
    let (override_var, fallback) = match provider {
        AgentProvider::Cursor => ("BRUHS_CURSOR_BIN", "cursor-agent"),
        AgentProvider::Antigravity => ("BRUHS_ANTIGRAVITY_BIN", "agy"),
        AgentProvider::Grok => ("BRUHS_GROK_BIN", "grok"),
        AgentProvider::OpenCode => ("BRUHS_OPENCODE_BIN", "opencode"),
        AgentProvider::Claude | AgentProvider::Codex | AgentProvider::Acp => {
            return Err(io::Error::other("Not an external provider."));
        }
    };
    if let Some(path) = std::env::var_os(override_var).filter(|path| !path.is_empty()) {
        return Ok(path.into());
    }
    if provider == AgentProvider::Grok {
        if let Some(path) = std::env::var_os("GROK_CLI_PATH").filter(|path| !path.is_empty()) {
            return Ok(path.into());
        }
        if let Some(base) = directories::BaseDirs::new() {
            let bundled = base.home_dir().join(".grok/bin/agent");
            if bundled.is_file() {
                return Ok(bundled);
            }
        }
    }
    Ok(fallback.into())
}

fn read_object(path: &Path) -> io::Result<Value> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let value: Value = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            if !value.is_object() {
                return Err(io::Error::other(format!(
                    "{} must contain a JSON object.",
                    path.display()
                )));
            }
            Ok(value)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(json!({})),
        Err(error) => Err(error),
    }
}

fn cursor_config(options: &Options, directory: &Path, command: &mut Command) -> io::Result<()> {
    let original = std::env::var_os("CURSOR_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_CONFIG_HOME").map(|path| PathBuf::from(path).join("cursor"))
        })
        .or_else(|| directories::BaseDirs::new().map(|base| base.home_dir().join(".cursor")))
        .ok_or_else(|| io::Error::other("Cursor configuration directory is unavailable."))?;
    let mut config = read_object(&original.join("cli-config.json"))?;
    config["version"] = json!(1);
    config["editor"] = config
        .get("editor")
        .cloned()
        .unwrap_or_else(|| json!({"vimMode":false}));
    config["approvalMode"] = json!("allowlist");
    if config.get("permissions").is_none() {
        config["permissions"] = json!({"allow":[],"deny":[]});
    }
    if !config["permissions"].is_object() {
        return Err(io::Error::other(
            "Cursor permissions must be a JSON object.",
        ));
    }
    let (allow, deny) = match options.mode {
        PermissionMode::Default | PermissionMode::Plan => (
            Vec::new(),
            vec!["Write(*)", "Write(**)", "Shell(*)", "Mcp(*:*)"],
        ),
        PermissionMode::AcceptEdits => (vec!["Write(**)"], vec!["Shell(*)", "Mcp(*:*)"]),
        PermissionMode::BypassPermissions => (Vec::new(), Vec::new()),
    };
    for (field, rules) in [("allow", allow), ("deny", deny)] {
        if config["permissions"].get(field).is_none() {
            config["permissions"][field] = json!([]);
        }
        let list = config["permissions"][field]
            .as_array_mut()
            .ok_or_else(|| io::Error::other("Cursor permission rules must be arrays."))?;
        list.extend(rules.into_iter().map(|rule| json!(rule)));
    }
    let mut mcp = read_object(&original.join("mcp.json"))?;
    if mcp.get("mcpServers").is_none() {
        mcp["mcpServers"] = json!({});
    }
    let servers = mcp["mcpServers"]
        .as_object_mut()
        .ok_or_else(|| io::Error::other("Cursor MCP configuration must be an object."))?;
    for (name, server) in &options.mcp {
        servers.insert(
            name.clone(),
            json!({"command":server.command,"args":server.args,"env":server.env}),
        );
    }
    std::fs::write(
        directory.join("cli-config.json"),
        serde_json::to_vec(&config)?,
    )?;
    std::fs::write(directory.join("mcp.json"), serde_json::to_vec(&mcp)?)?;
    command.env("CURSOR_CONFIG_DIR", directory);
    Ok(())
}

fn opencode_config(options: &Options, command: &mut Command) -> io::Result<String> {
    let mut config = std::env::var("OPENCODE_CONFIG_CONTENT")
        .ok()
        .map(|text| serde_json::from_str::<Value>(&text))
        .transpose()
        .map_err(io::Error::other)?
        .unwrap_or_else(|| json!({}));
    if !config.is_object() {
        return Err(io::Error::other(
            "OPENCODE_CONFIG_CONTENT must be a JSON object.",
        ));
    }
    let mut permissions = match options.mode {
        PermissionMode::Default | PermissionMode::Plan => {
            json!({"*":"deny","read":"allow","glob":"allow","grep":"allow","list":"allow"})
        }
        PermissionMode::AcceptEdits => {
            json!({"*":"deny","read":"allow","glob":"allow","grep":"allow","list":"allow","edit":"allow","write":"allow","patch":"allow"})
        }
        PermissionMode::BypassPermissions => json!({"*":"allow"}),
    };
    for inherited in &options.permission_defaults {
        preserve_denials(&mut permissions, inherited);
    }
    if let Some(tools) = options.agent_defaults["tools"].as_object() {
        for (name, enabled) in tools {
            if enabled == false {
                permissions[name] = json!("deny");
            }
        }
    }
    if config.get("agent").is_none() {
        config["agent"] = json!({});
    }
    let agents = config["agent"]
        .as_object_mut()
        .ok_or_else(|| io::Error::other("OpenCode agent configuration must be a JSON object."))?;
    let agent = format!("bruhs-{}", uuid::Uuid::new_v4());
    let mut settings = options.agent_defaults.clone();
    settings["mode"] = json!("primary");
    settings["permission"] = permissions.clone();
    agents.insert(agent.clone(), settings);
    if config.get("mcp").is_none() {
        config["mcp"] = json!({});
    }
    let servers = config["mcp"]
        .as_object_mut()
        .ok_or_else(|| io::Error::other("OpenCode MCP configuration must be an object."))?;
    for (name, server) in &options.mcp {
        let command: Vec<&str> = std::iter::once(server.command.as_str())
            .chain(server.args.iter().map(String::as_str))
            .collect();
        servers.insert(
            name.clone(),
            json!({"type":"local","command":command,"environment":server.env,"enabled":true}),
        );
    }
    command.env("OPENCODE_CONFIG_CONTENT", config.to_string());
    command.env("OPENCODE_PERMISSION", permissions.to_string());
    Ok(agent)
}

// Match the existing provider status probes: resolving local CLI metadata
// should not hold a turn open indefinitely when a plugin or config is broken.
const CONFIG_TIMEOUT: Duration = Duration::from_secs(15);

fn preserve_denials(policy: &mut Value, inherited: &Value) {
    let Some(rules) = inherited.as_object() else {
        return;
    };
    if rules.get("*").is_some_and(|rule| rule == "deny") {
        if let Some(actions) = policy.as_object_mut() {
            for action in actions.values_mut() {
                *action = json!("deny");
            }
        }
    }
    for (tool, rule) in rules {
        if rule == "deny" {
            policy[tool] = json!("deny");
        } else if let Some(patterns) = rule.as_object() {
            let denied: Vec<_> = patterns
                .iter()
                .filter(|(_, action)| *action == "deny")
                .collect();
            if denied.is_empty() {
                continue;
            }
            if !policy[tool].is_object() {
                let fallback = policy[tool]
                    .as_str()
                    .or_else(|| policy["*"].as_str())
                    .unwrap_or("deny")
                    .to_string();
                policy[tool] = json!({"*":fallback});
            }
            for (pattern, _) in denied {
                policy[tool][pattern] = json!("deny");
            }
        }
    }
}

fn configured_opencode_agent(config: &Value) -> io::Result<Value> {
    let name = config["default_agent"].as_str().unwrap_or("build");
    let mut agent = config["agent"][name].clone();
    if agent.is_null() && matches!(name, "build" | "plan") {
        agent = json!({});
    }
    if !agent.is_object()
        || agent["disable"] == true
        || agent["hidden"] == true
        || agent["mode"] == "subagent"
    {
        return Err(io::Error::other(
            "OpenCode's default agent is unavailable. Configure an enabled primary default_agent in OpenCode and retry.",
        ));
    }
    Ok(agent)
}

async fn opencode_agent_defaults(
    options: &Options,
    turn: &TurnContext,
) -> io::Result<(Value, Value)> {
    let mut command = Command::new(executable(AgentProvider::OpenCode)?);
    command
        .args(["debug", "config"])
        .current_dir(&options.cwd)
        .stdin(Stdio::null());
    process::apply_env(
        &mut command,
        true,
        &crate::connections::PRIVATE_ENV.map(String::from),
        &[],
    );
    let mut child = process::CliProcess::spawn("OpenCode configuration", &mut command)?;
    let mut output = String::new();
    let ending = loop {
        match child.next_line(&turn.tx).await {
            Ok(line) => {
                output.push_str(&line);
                output.push('\n');
            }
            Err(ending) => break ending,
        }
    };
    let cancelled = matches!(ending, process::Ending::ConsumerGone);
    if child.finish(ending).await.is_some() || cancelled {
        return Err(io::Error::other(
            "Could not resolve OpenCode configuration. Check `opencode debug config` in a terminal and retry.",
        ));
    }
    let config: Value = serde_json::from_str(&output).map_err(|_| {
        io::Error::other(
            "OpenCode returned invalid configuration metadata. Update its CLI and retry.",
        )
    })?;
    Ok((
        configured_opencode_agent(&config)?,
        config["permission"].clone(),
    ))
}

pub(super) async fn stream(mut options: Options, turn: TurnContext) {
    if options.provider == AgentProvider::Acp {
        acp::stream(options, turn).await;
        return;
    }
    if turn.tx.is_closed() {
        turn.finish(false);
        return;
    }
    let provider = options.provider;
    if provider == AgentProvider::OpenCode && !options.unattended {
        match tokio::time::timeout(CONFIG_TIMEOUT, opencode_agent_defaults(&options, &turn)).await {
            Ok(Ok((agent, permission))) => {
                options.permission_defaults = vec![permission, agent["permission"].clone()];
                options.agent_defaults = agent;
            }
            Ok(Err(error)) => {
                turn.send_start_error(error.to_string()).await;
                return;
            }
            Err(_) => {
                turn.send_start_error("OpenCode configuration resolution timed out. Check `opencode debug config` in a terminal and retry.".into()).await;
                return;
            }
        }
    }
    let mut parser = Parser::new(provider, &options.model, options.mode, &options.mcp);
    let prepared = tokio::task::spawn_blocking(move || options.prepare()).await;
    let mut prepared = match prepared {
        Ok(Ok(prepared)) => prepared,
        Ok(Err(error)) => {
            turn.send_start_error(error.to_string()).await;
            return;
        }
        Err(error) => {
            turn.send_start_error(format!("Could not prepare {}: {error}", provider.label()))
                .await;
            return;
        }
    };
    if turn.tx.is_closed() {
        turn.finish(false);
        return;
    }
    let mut process = match process::CliProcess::spawn(provider.label(), &mut prepared.command) {
        Ok(process) => process,
        Err(error) => {
            turn.send_start_error(format!("Could not start {}: {error}. Install its CLI and sign in, or set BRUHS_{}_BIN to its executable.", provider.label(), provider.as_str().to_ascii_uppercase())).await;
            return;
        }
    };
    turn.register_process(process.pid());
    let write = async {
        let mut stdin = process
            .take_stdin()
            .ok_or_else(|| io::Error::other("CLI stdin was not piped."))?;
        stdin.write_all(&prepared.input).await?;
        stdin.shutdown().await
    };
    let written = tokio::select! {
        () = turn.tx.closed() => None,
        result = write => Some(result),
    };
    let (ending, alive) = match written {
        None => (process::Ending::ConsumerGone, false),
        Some(Err(error)) => (
            process::Ending::Failed(format!(
                "Sending the prompt to {} failed: {error}",
                provider.label()
            )),
            true,
        ),
        Some(Ok(())) => pump(&mut process, &mut parser, &turn).await,
    };
    let failure = process.finish(ending).await;
    if alive {
        if let Some(failure) = failure {
            if !parser.reported_error {
                turn.send_start_error(failure.to_string()).await;
            }
        } else if provider == AgentProvider::OpenCode
            && !parser.terminal
            && parser.completed
            && !parser.reported_error
        {
            let usage = std::mem::take(&mut parser.usage);
            let result = parser.result(false, parser.text.clone(), usage, 0);
            turn.remember_session(&result);
            let _ = turn.forward(result).await;
        } else if !parser.terminal && !parser.reported_error {
            turn.send_start_error(format!("{} closed its stream without a completed result. Update the CLI and retry; the turn may have been interrupted.", provider.label())).await;
        }
    }
    turn.finish(alive);
}

async fn pump(
    process: &mut process::CliProcess,
    parser: &mut Parser,
    turn: &TurnContext,
) -> (process::Ending, bool) {
    loop {
        let line = match process.next_line(&turn.tx).await {
            Ok(line) => line,
            Err(ending) => {
                let alive = !matches!(ending, process::Ending::ConsumerGone);
                return (ending, alive);
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let value = match serde_json::from_str::<Value>(&line) {
            Ok(value) => value,
            Err(error) => {
                return (
                    process::Ending::Failed(format!(
                        "{} emitted invalid JSON: {error}. Update its CLI and retry.",
                        parser.provider.label()
                    )),
                    true,
                );
            }
        };
        for event in parser.parse(value) {
            parser.reported_error |= super::is_error_event(&event);
            turn.remember_session(&event);
            if !turn.forward(event).await {
                return (process::Ending::ConsumerGone, false);
            }
        }
    }
}

struct Parser {
    provider: AgentProvider,
    model: String,
    session: String,
    mode: PermissionMode,
    mcp: Vec<String>,
    text: String,
    terminal: bool,
    reported_error: bool,
    partial: bool,
    completed: bool,
    usage: UsageInfo,
    tools: HashSet<String>,
    parts: HashSet<String>,
}

impl Parser {
    fn new(
        provider: AgentProvider,
        model: &str,
        mode: PermissionMode,
        mcp: &HashMap<String, Mcp>,
    ) -> Self {
        Self {
            provider,
            model: model.into(),
            session: String::new(),
            mode,
            mcp: mcp.keys().cloned().collect(),
            text: String::new(),
            terminal: false,
            reported_error: false,
            partial: false,
            completed: false,
            usage: UsageInfo::default(),
            tools: HashSet::new(),
            parts: HashSet::new(),
        }
    }

    fn init(&self, tools: Vec<String>) -> QueryEvent {
        let unavailable = self.provider != AgentProvider::OpenCode;
        let status = if self.mode != PermissionMode::BypassPermissions {
            "disabled by access mode"
        } else {
            "configured"
        };
        let mcp_servers = if unavailable {
            vec![McpServerInfo {
                name: "bruhs integrations".into(),
                status: if self.provider == AgentProvider::Cursor {
                    "unavailable: runtime requires persistent MCP server approval"
                } else {
                    "unavailable: runtime has no per-turn MCP configuration"
                }
                .into(),
            }]
        } else {
            self.mcp
                .iter()
                .map(|name| McpServerInfo {
                    name: name.clone(),
                    status: status.into(),
                })
                .collect()
        };
        QueryEvent::Init {
            session_id: self.session.clone(),
            model: Some(self.model.clone()),
            tools,
            mcp_servers,
        }
    }

    fn result(
        &mut self,
        is_error: bool,
        result: String,
        usage: UsageInfo,
        duration_ms: u64,
    ) -> QueryEvent {
        self.terminal = true;
        QueryEvent::Result {
            result,
            session_id: self.session.clone(),
            duration_ms,
            usage,
            is_error,
        }
    }

    fn text(&mut self, text: &str, thinking: bool) -> QueryEvent {
        if thinking {
            QueryEvent::Thinking(text.into())
        } else {
            self.text.push_str(text);
            QueryEvent::Text(text.into())
        }
    }

    fn parse(&mut self, value: Value) -> Vec<QueryEvent> {
        if self.provider == AgentProvider::Antigravity {
            return self.antigravity(value);
        }
        if self.provider == AgentProvider::OpenCode {
            return self.opencode(value);
        }
        self.messages(value)
    }

    fn messages(&mut self, value: Value) -> Vec<QueryEvent> {
        if value
            .get("parent_tool_use_id")
            .is_some_and(|parent| !parent.is_null())
        {
            return vec![QueryEvent::Raw(value)];
        }
        match string(&value, "type") {
            "system" if string(&value, "subtype") == "init" => {
                self.session = string(&value, "session_id").into();
                vec![self.init(strings(&value["tools"]))]
            }
            "stream_event" => {
                let event = &value["event"];
                match string(event, "type") {
                    "message_start" => {
                        self.partial = false;
                        Vec::new()
                    }
                    "content_block_delta" => match string(&event["delta"], "type") {
                        "text_delta" => {
                            self.partial = true;
                            vec![self.text(string(&event["delta"], "text"), false)]
                        }
                        "thinking_delta" => {
                            self.partial = true;
                            vec![self.text(string(&event["delta"], "thinking"), true)]
                        }
                        _ => Vec::new(),
                    },
                    "message_delta" => vec![QueryEvent::Usage(usage(&event["usage"]))],
                    _ => Vec::new(),
                }
            }
            "assistant" | "user" => {
                let cursor = self.provider == AgentProvider::Cursor;
                let delta = cursor
                    && value.get("timestamp_ms").is_some()
                    && value.get("model_call_id").is_none();
                let emit_text = delta || !self.partial;
                if delta {
                    self.partial = true;
                }
                let mut events = Vec::new();
                for block in value["message"]["content"].as_array().into_iter().flatten() {
                    match string(block, "type") {
                        "text" if emit_text => events.push(self.text(string(block, "text"), false)),
                        "thinking" if emit_text => {
                            events.push(self.text(string(block, "thinking"), true))
                        }
                        "tool_use" => events.push(QueryEvent::ToolCall {
                            id: string(block, "id").into(),
                            tool: string(block, "name").into(),
                            input: block["input"].clone(),
                        }),
                        "tool_result" => events.push(QueryEvent::ToolResult {
                            tool_use_id: string(block, "tool_use_id").into(),
                            content: content(&block["content"]),
                            is_error: block["is_error"].as_bool().unwrap_or(false),
                        }),
                        _ => {}
                    }
                }
                events
            }
            "tool_call" if self.provider == AgentProvider::Cursor => {
                let Some((name, tool)) = value["tool_call"]
                    .as_object()
                    .and_then(|tools| tools.iter().next())
                else {
                    return vec![QueryEvent::Raw(value)];
                };
                let id = string(&value, "call_id").to_string();
                if string(&value, "subtype") == "started" {
                    self.partial = false;
                    vec![QueryEvent::ToolCall {
                        id,
                        tool: name.clone(),
                        input: tool["args"].clone(),
                    }]
                } else {
                    vec![QueryEvent::ToolResult {
                        tool_use_id: id,
                        content: content(&tool["result"]),
                        is_error: tool["result"].get("error").is_some(),
                    }]
                }
            }
            "result" => {
                let id = string(&value, "session_id");
                if !id.is_empty() {
                    self.session = id.into();
                }
                let error = value["is_error"].as_bool().unwrap_or(false)
                    || string(&value, "subtype").starts_with("error");
                let text = if error && string(&value, "result").is_empty() {
                    content(&value["errors"])
                } else {
                    string(&value, "result").into()
                };
                vec![self.result(
                    error,
                    text,
                    usage(&value["usage"]),
                    value["duration_ms"].as_u64().unwrap_or_default(),
                )]
            }
            "error" => vec![QueryEvent::Error(content(&value))],
            _ => vec![QueryEvent::Raw(value)],
        }
    }

    fn antigravity(&mut self, value: Value) -> Vec<QueryEvent> {
        match string(&value, "event") {
            "init" => {
                self.session = string(&value, "conversation_id").into();
                vec![self.init(strings(&value["init"]["tools"]))]
            }
            "step_update" => {
                let step = &value["step_update"];
                let mut events = Vec::new();
                if string(step, "step_type") == "agent_response"
                    && !string(step, "text_delta").is_empty()
                {
                    events.push(self.text(string(step, "text_delta"), false));
                }
                if matches!(string(step, "step_type"), "thought" | "thinking") {
                    let delta = if !string(step, "thinking_delta").is_empty() {
                        string(step, "thinking_delta")
                    } else {
                        string(step, "text_delta")
                    };
                    if !delta.is_empty() {
                        events.push(self.text(delta, true));
                    }
                }
                if string(step, "step_type") == "tool" {
                    let id = format!("step-{}", step["step_index"]);
                    let info = &step["tool_info"];
                    if self.tools.insert(id.clone()) {
                        events.push(QueryEvent::ToolCall {
                            id: id.clone(),
                            tool: string(info, "name").into(),
                            input: info["parameters"].clone(),
                        });
                    }
                    if string(step, "state") == "DONE" {
                        events.push(QueryEvent::ToolResult {
                            tool_use_id: id,
                            content: content(&info["output"]),
                            is_error: info.get("error").is_some(),
                        });
                    }
                }
                if step.get("usage").is_some() {
                    events.push(QueryEvent::Usage(agy_usage(&step["usage"])));
                }
                events
            }
            "result" => {
                let result = &value["result"];
                self.session = string(result, "conversation_id").into();
                let error = string(result, "status") != "SUCCESS";
                let text = if error {
                    format!(
                        "{} {}",
                        string(result, "response"),
                        content(&result["error"])
                    )
                    .trim()
                    .into()
                } else {
                    string(result, "response").into()
                };
                let ms = result["duration_seconds"]
                    .as_f64()
                    .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
                    .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok())
                    .map_or(0, |duration| {
                        u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
                    });
                vec![self.result(error, text, agy_usage(&result["usage"]), ms)]
            }
            _ => vec![QueryEvent::Raw(value)],
        }
    }

    fn opencode(&mut self, value: Value) -> Vec<QueryEvent> {
        let mut events = Vec::new();
        if self.session.is_empty() && !string(&value, "sessionID").is_empty() {
            self.session = string(&value, "sessionID").into();
            events.push(self.init(Vec::new()));
        }
        let part = &value["part"];
        match string(&value, "type") {
            "step_start" => {
                self.completed = false;
            }
            "text" | "reasoning" => {
                let id = string(part, "id");
                if id.is_empty() || self.parts.insert(id.into()) {
                    events.push(
                        self.text(string(part, "text"), string(&value, "type") == "reasoning"),
                    );
                }
            }
            "tool_use" => {
                let id = string(part, "callID").to_string();
                if self.tools.insert(id.clone()) {
                    events.push(QueryEvent::ToolCall {
                        id: id.clone(),
                        tool: string(part, "tool").into(),
                        input: part["state"]["input"].clone(),
                    });
                }
                let state = &part["state"];
                let error = string(state, "status") == "error";
                events.push(QueryEvent::ToolResult {
                    tool_use_id: id,
                    content: content(&state[if error { "error" } else { "output" }]),
                    is_error: error,
                });
            }
            "step_finish" => {
                self.completed = matches!(string(part, "reason"), "stop" | "end_turn");
                let usage = opencode_usage(&part["tokens"]);
                self.usage.input_tokens =
                    self.usage.input_tokens.saturating_add(usage.input_tokens);
                self.usage.output_tokens =
                    self.usage.output_tokens.saturating_add(usage.output_tokens);
                self.usage.cache_read_input_tokens = self
                    .usage
                    .cache_read_input_tokens
                    .saturating_add(usage.cache_read_input_tokens);
                self.usage.cache_creation_input_tokens = self
                    .usage
                    .cache_creation_input_tokens
                    .saturating_add(usage.cache_creation_input_tokens);
                events.push(QueryEvent::Usage(usage));
            }
            "error" => events.push(QueryEvent::Error(content(&value["error"]))),
            _ => events.push(QueryEvent::Raw(value)),
        }
        events
    }
}

fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}
fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(String::from)
        .collect()
}
fn content(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        Value::Array(parts) => parts
            .iter()
            .map(|part| part.get("text").map_or_else(|| content(part), content))
            .collect::<Vec<_>>()
            .join("\n"),
        other => other.to_string(),
    }
}
fn usage(value: &Value) -> UsageInfo {
    serde_json::from_value(value.clone()).unwrap_or_default()
}
fn agy_usage(value: &Value) -> UsageInfo {
    UsageInfo {
        input_tokens: value["input_tokens"]
            .as_u64()
            .unwrap_or_default()
            .saturating_sub(value["cache_read_tokens"].as_u64().unwrap_or_default()),
        output_tokens: value["output_tokens"].as_u64().unwrap_or_default(),
        cache_read_input_tokens: value["cache_read_tokens"].as_u64().unwrap_or_default(),
        ..UsageInfo::default()
    }
}
fn opencode_usage(value: &Value) -> UsageInfo {
    UsageInfo {
        input_tokens: value["input"].as_u64().unwrap_or_default(),
        output_tokens: value["output"].as_u64().unwrap_or_default(),
        cache_read_input_tokens: value["cache"]["read"].as_u64().unwrap_or_default(),
        cache_creation_input_tokens: value["cache"]["write"].as_u64().unwrap_or_default(),
        context_tokens: None,
    }
}

#[cfg(test)]
#[path = "external/tests.rs"]
mod tests;
