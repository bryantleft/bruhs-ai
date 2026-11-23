#![cfg(unix)]
#![expect(clippy::unwrap_used, reason = "integration test helpers")]
use bruhs_core::agent::health::account::{self, Account, Action, Event, Kind};
use std::{io, os::unix::fs::PermissionsExt, path::Path, process::Command};
use tokio::sync::mpsc;

const CLI: &str = r#"#!/usr/bin/env python3
import json, os, sys, pathlib
root=pathlib.Path(os.environ["BRUHS_FIXTURE_ACCOUNT"])
state=root/"signed"
if "--login-terminal" in sys.argv:
    assert sys.argv[-1]=="literal ; arg" and os.isatty(0) and os.environ["PTY_LOGIN"]=="yes"
    print("Fixture sign-in code:",flush=True)
    if input().strip()!="work": sys.exit(4)
    state.touch()
    sys.exit(0)
def send(v): print(json.dumps({"jsonrpc":"2.0",**v}),flush=True)
def result(value): send({"id":req["id"],"result":value})
def fail(code): send({"id":req["id"],"error":{"code":code,"message":"fixture"}})
methods=[{"id":"device","name":"Device login","type":"agent"},{"id":"terminal","name":"Terminal login","type":"terminal","args":["--login-terminal","literal ; arg"],"env":{"PTY_LOGIN":"yes"}},{"id":"browser","name":"Browser login","type":"agent"},{"id":"key","name":"API key","type":"env_var","vars":[{"name":"FIXTURE_KEY"}],"link":"https://example.invalid/account"},{"id":"unverified","name":"Unverified"}]
for line in sys.stdin:
    req=json.loads(line);method=req["method"];p=req["params"]
    with (root/"calls").open("a") as f: print(method,file=f)
    if method=="initialize":result({"protocolVersion":1,"authMethods":methods,"agentCapabilities":{"auth":{"logout":{}}}})
    elif method=="session/new":
        if state.exists() or (root/"anonymous").exists() or os.getenv("FIXTURE_KEY")=="configured": result({"sessionId":"verified"})
        else:fail(-32000)
    elif method=="logout":
        state.unlink(missing_ok=True);result({})
    elif method=="authenticate":
        if p["methodId"]=="browser":
            (root/"browser-pid").write_text(str(os.getpid()))
            send({"id":"url","method":"elicitation/create","params":{"mode":"url","url":(root/"url").read_text(),"elicitationId":"login"}})
            response=json.loads(next(sys.stdin))
            if response.get("result",{}).get("action")!="accept":fail(-32000);continue
        if p["methodId"]!="unverified":state.touch()
        result({})
    else:fail(-32601)
"#;

async fn attempt(action: Action, answer: Option<bool>, code: &str) -> io::Result<Account> {
    let (tx, mut rx) = mpsc::channel(4);
    let job = tokio::spawn(account::run("work", action, tx));
    while let Some(event) = rx.recv().await {
        match event {
            Event::Verifying => (),
            Event::Browser { url, response } => {
                assert_eq!(url, "https://example.invalid/login");
                response
                    .send(answer.expect("unexpected URL request"))
                    .unwrap();
            }
            Event::Terminal(manager, info) => {
                manager
                    .write(&info.id, format!("{code}\n").as_bytes())
                    .unwrap();
            }
        }
    }
    job.await.unwrap()
}

async fn verify(root: &Path) {
    let cli = root.join("my agent");
    std::fs::write(&cli, CLI).unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    let config = serde_json::json!({"acp":[{"id":"work","name":"Work","enabled":true,"command":cli,"args":["--acp"]}]});
    std::fs::write(
        root.join("agents.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    let inspect = || attempt(Action::Discover, None, "");
    let signin = |id: &str| Action::SignIn(id.into());
    let signout = || attempt(Action::SignOut, None, "");
    let status = inspect().await.unwrap();
    assert!(!status.ready && !status.confirmed && status.logout && status.methods.len() == 5);
    assert!(status.methods[1].kind == Kind::Terminal);
    assert_eq!(status.methods[3].variables, ["FIXTURE_KEY"]);
    assert!(attempt(signin("unverified"), None, "").await.is_err());
    assert!(attempt(signin("missing"), None, "").await.is_err());
    assert!(attempt(signin("device"), None, "").await.unwrap().confirmed);
    assert!(inspect().await.unwrap().confirmed);
    assert!(!signout().await.unwrap().confirmed);
    std::fs::write(root.join("url"), "file:///private/example").unwrap();
    assert!(attempt(signin("browser"), None, "").await.is_err());
    std::fs::write(root.join("url"), "https://example.invalid/login").unwrap();
    assert!(attempt(signin("browser"), Some(false), "").await.is_err());
    assert!(
        attempt(signin("browser"), Some(true), "")
            .await
            .unwrap()
            .confirmed
    );
    signout().await.unwrap();
    let (tx, mut rx) = mpsc::channel(4);
    let pending = tokio::spawn(account::run("work", signin("browser"), tx));
    let Some(Event::Browser { response, .. }) = rx.recv().await else {
        panic!("expected consent request")
    };
    assert!(
        inspect().await.is_err(),
        "concurrent discovery must not change account state"
    );
    pending.abort();
    assert!(matches!(pending.await,Err(err) if err.is_cancelled()));
    drop(response);
    assert!(!inspect().await.unwrap().ready);
    assert!(attempt(signin("terminal"), None, "wrong").await.is_err());
    assert!(
        attempt(signin("terminal"), None, "work")
            .await
            .unwrap()
            .confirmed
    );
    signout().await.unwrap();
    std::fs::write(root.join("anonymous"), "").unwrap();
    assert!(inspect().await.unwrap().ready);
    assert!(attempt(signin("key"), None, "").await.is_err());
    assert!(!inspect().await.unwrap().confirmed);
    std::fs::remove_file(root.join("anonymous")).unwrap();
    let mut keyed = config;
    keyed["acp"][0]["environment"] =
        serde_json::json!({"FIXTURE_KEY":{"kind":"plain","value":"configured"}});
    std::fs::write(
        root.join("agents.json"),
        serde_json::to_vec(&keyed).unwrap(),
    )
    .unwrap();
    let active = inspect().await.unwrap();
    assert!(active.ready && !active.confirmed);
    assert!(attempt(signin("key"), None, "").await.unwrap().confirmed);
    assert!(!std::fs::read_to_string(root.join("calls"))
        .unwrap()
        .contains("session/prompt"));
}

#[test]
fn account_discovery_sign_in_consent_terminal_and_logout() {
    if let Some(root) = std::env::var_os("BRUHS_FIXTURE_ACCOUNT") {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(verify(Path::new(&root)));
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let result = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "account_discovery_sign_in_consent_terminal_and_logout",
            "--nocapture",
        ])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir.path())
        .env("BRUHS_FIXTURE_ACCOUNT", dir.path())
        .env("BRUHS_WORKBENCH_DIR", dir.path())
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
