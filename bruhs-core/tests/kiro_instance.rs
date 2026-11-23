//! A separate process keeps provider settings and CLI credentials out of the user's profile.
#![cfg(unix)]
#![expect(clippy::unwrap_used, reason = "integration test helpers")]

use bruhs_core::{
    agent::{health, AgentOrchestrator, AgentTurn},
    agent_extensions::{availability::ProviderAvailability, instances, kiro::Catalog},
};
use bruhs_models::{
    agent::{AgentProvider, PermissionMode},
    agent_extensions::{AcpInstance, AcpVariable},
    ids::ThreadId,
};
use futures_util::StreamExt;
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

const CHILD: &str = "BRUHS_KIRO_TEST_ROOT";
const CLI: &str = r#"#!/usr/bin/env python3
import json, os, pathlib, sys, time
account = os.environ["FIXTURE_PROFILE"]
args = sys.argv[1:]
if args == ["--version"]:
    print("kiro-cli 2.27.0"); sys.exit()
if args == ["whoami", "--format", "json"]:
    print('{"account":null}' if account == "offline" else '{"accountType":"ApiKey"}')
    sys.exit(1 if account == "offline" else 0)
if args == ["chat", "--list-models", "--format", "json"]:
    models=[{"model_id":"auto","model_name":"Automatic"},{"model_id":"fast","model_name":"Fast model"}]
    if account=="options":models.append({"model_id":"kimi-k3","model_name":"Kimi K3"})
    print(json.dumps({"default_model":"auto","models":models})); sys.exit()
assert args == ["acp", "--agent-engine=v3", "--auth-method=cli"], args
def send(message):
    print(json.dumps(message), flush=True)
def options():
    extra=[{"id":"mode","type":"select","currentValue":agent,"options":[{"value":"main","name":"Default agent"},{"value":"review","name":"Reviewer","description":"Checks a change."}]}]
    if model=="kimi-k3":extra.append({"id":"effortLevel","type":"select","currentValue":effort,"options":[{"value":"low"},{"value":"high"},{"value":"max"}]})
    return extra+[{"id":"model","type":"select","currentValue":model,"options":[{"value":"auto"},{"value":"fast"}]},
            {"id":"autopilot","type":"select","currentValue":access,"options":[{"value":"off"},{"value":"on"}]}]
model, access, agent, effort = "fast", "on", "main", "max"
for line in sys.stdin:
    request = json.loads(line)
    method, params = request["method"], request["params"]
    with open(pathlib.Path(os.environ["BRUHS_KIRO_TEST_ROOT"])/"methods", "a") as log:
        print(account, method, params.get("value", ""), file=log)
    if method == "initialize": result = {"protocolVersion":1,"agentCapabilities":{"loadSession":True}}
    elif method=="_kiro/config/template":result={"configOptions":options()[:1]} if account=="options" else {}
    elif method in ["session/new", "session/load"]:
        assert params.get("sessionId", account) == account
        result = {"sessionId":account,"configOptions":options()}
    elif method == "session/set_config_option":
        if params["configId"] == "mode":
            assert params["value"] in ["main","review"]
            agent,model,effort,access=params["value"],"auto","max","on"
        elif params["configId"] == "effortLevel":
            assert model=="kimi-k3" and params["value"] in ["low","high","max"]
            effort=params["value"]
        elif params["configId"] == "model":
            before=options()
            model,effort=params["value"],"max"
            if model=="kimi-k3":
                send({"jsonrpc":"2.0","id":request["id"],"result":{"configOptions":before}})
                time.sleep(.055)
                send({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":account,"update":{"sessionUpdate":"config_option_update","configOptions":options()}}})
                continue
        else:
            assert params["configId"] == "autopilot"
            if account != "refuses": access = params["value"]
        result = {"configOptions":options()}
    elif method == "session/prompt":
        send({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":account,"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":account+"/"+model+"/"+access+"/"+agent+"/"+effort}}}})
        result = {"stopReason":"end_turn"}
    else: raise ValueError(method)
    send({"jsonrpc":"2.0","id":request["id"],"result":result})
"#;

async fn turn(
    host: &AgentOrchestrator,
    id: ThreadId,
    model: &str,
    mode: PermissionMode,
    session: Option<&str>,
) -> String {
    let turn = AgentTurn::new(id, "Hello", ".", model, AgentProvider::Acp, mode)
        .resume(session.map(str::to_owned));
    let mut events = host.send_message(turn, None).unwrap();
    let mut result = String::new();
    while let Some(event) = events.next().await {
        result.push_str(&format!("{event:?}"));
    }
    result
}

async fn verify(root: &Path) {
    let cli = root.join("kiro fixture");
    fs::write(&cli, CLI).unwrap();
    fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).unwrap();
    for profile in ["blue", "green", "offline", "refuses", "options"] {
        let mut instance = AcpInstance::kiro(profile.into(), profile.into(), cli.to_str().unwrap());
        instance.environment.insert(
            "FIXTURE_PROFILE".into(),
            AcpVariable::Plain {
                value: profile.into(),
            },
        );
        instances::edit(None, Some(&instance)).unwrap();
    }
    for provider in [
        AgentProvider::Cursor,
        AgentProvider::Antigravity,
        AgentProvider::Grok,
        AgentProvider::OpenCode,
    ] {
        ProviderAvailability::set_enabled(provider, false).unwrap();
    }
    let rows = health::external_provider_health(&ProviderAvailability::load().unwrap()).await;
    let blue = rows.iter().find(|row| row.id == "acp/blue").unwrap();
    assert_eq!(blue.status, health::Status::SignedIn);
    assert_eq!(blue.model_count, Some(2));
    assert_eq!(blue.version.as_deref(), Some("2.27.0"));
    assert_eq!(
        rows.iter()
            .find(|row| row.id == "acp/offline")
            .unwrap()
            .status,
        health::Status::SignedOut
    );
    let instance = bruhs_core::agent_extensions::load_config()
        .unwrap()
        .acp
        .remove(0);
    assert_eq!(
        Catalog::load(&instance).choices(&instance)[1],
        ("fast".into(), "Fast model".into())
    );
    let host = AgentOrchestrator::new(Default::default());
    let id = ThreadId::new_v4();
    let first = turn(
        &host,
        id.clone(),
        "acp/blue/default",
        PermissionMode::Default,
        None,
    )
    .await;
    assert!(first.contains("blue/auto/off"), "{first}");
    let resumed = turn(
        &host,
        id.clone(),
        "acp/blue/unlisted",
        PermissionMode::BypassPermissions,
        Some("acp:blue:blue"),
    )
    .await;
    assert!(resumed.contains("blue/unlisted/on"), "{resumed}");
    let other = turn(&host, id, "acp/green/fast", PermissionMode::Default, None).await;
    assert!(other.contains("green/fast/off"), "{other}");
    let failed = turn(
        &host,
        ThreadId::new_v4(),
        "acp/refuses/fast",
        PermissionMode::Default,
        None,
    )
    .await;
    assert!(failed.contains("did not confirm"), "{failed}");
    let saved = bruhs_core::agent_extensions::load_config()
        .unwrap()
        .acp
        .into_iter()
        .find(|i| i.id == "options")
        .unwrap();
    let catalog = Catalog::load(&saved);
    assert_eq!(catalog.agents.current.as_deref(), Some("main"));
    assert_eq!(
        catalog.agents.choices[1].description.as_deref(),
        Some("Checks a change.")
    );
    for (agent, level, expected) in [
        ("review", "high", "review/high"),
        ("unlisted", "none", "main/max"),
    ] {
        let mut turn = AgentTurn::new(
            ThreadId::new_v4(),
            "Check",
            ".",
            "acp/options/kimi-k3",
            AgentProvider::Acp,
            PermissionMode::Default,
        );
        turn.kiro_selection = bruhs_core::agent_extensions::kiro::Selection {
            agent: Some(agent.into()),
            effort: Some(level.into()),
        };
        let mut events = host.send_message(turn, None).unwrap();
        let mut output = String::new();
        while let Some(event) = events.next().await {
            output.push_str(&format!("{event:?}"));
        }
        assert!(
            output.contains(&format!("options/kimi-k3/off/{expected}")),
            "{output}"
        );
    }
    let log = fs::read_to_string(root.join("methods")).unwrap();
    assert!(log.contains("blue session/load") && log.contains("green session/new"));
    assert!(!log.contains("green session/load") && !log.contains("refuses session/prompt"));
}

#[test]
fn named_kiro_instances_authenticate_select_models_and_confirm_access() {
    if let Some(root) = std::env::var_os(CHILD) {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(verify(Path::new(&root)));
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let result = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("named_kiro_instances_authenticate_select_models_and_confirm_access")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", directory.path())
        .env(CHILD, directory.path())
        .env("BRUHS_WORKBENCH_DIR", directory.path())
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
