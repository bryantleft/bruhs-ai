use super::*;

fn profile() -> RemoteHostProfile {
    RemoteHostProfile {
        id: Uuid::new_v4(),
        name: "Synthetic SSH host".into(),
        host: "fixture".into(),
        ssh_port: 22,
        local_port: 45001,
        remote_port: 45000,
        connect_timeout_seconds: 5,
    }
}

async fn wait_state(
    manager: &RemoteHostManager,
    id: Uuid,
    expected: RemoteHostState,
) -> RemoteHost {
    let mut changes = manager.changes();
    tokio::time::timeout(crate::capture_client::REQUEST_TIMEOUT, async {
        loop {
            let host = manager
                .snapshot()
                .await
                .hosts
                .into_iter()
                .find(|host| host.profile.id == id)
                .unwrap();
            if host.state == expected {
                return host;
            }
            assert!(
                host.state != RemoteHostState::Failed || expected == RemoteHostState::Failed,
                "Unexpected failure: {:?}",
                host.error
            );
            changes.changed().await.unwrap();
        }
    })
    .await
    .expect("SSH fixture state deadline")
}

#[test]
fn tunnel_args_keep_host_trust_and_literal_loopback_forwarding() {
    let profile = profile();
    let args = process::tunnel_args(&profile, Path::new("/private/control"));
    assert!(args
        .windows(2)
        .any(|args| args == ["-o", "StrictHostKeyChecking=yes"]));
    assert!(args.windows(2).any(|args| args == ["-o", "BatchMode=yes"]));
    assert!(args
        .windows(2)
        .any(|args| args == ["-L", "127.0.0.1:45001:127.0.0.1:45000"]));
    assert_eq!(&args[args.len() - 2..], ["--", "fixture"]);
    assert!(!args
        .iter()
        .any(|arg| arg == "-f" || arg == "StrictHostKeyChecking=no"));
}

#[tokio::test]
async fn restart_records_interruption_without_spawning_or_killing_a_saved_pid() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("hosts.json");
    let profile = profile();
    let host = RemoteHost {
        local_url: "javascript:ignored".into(),
        profile: profile.clone(),
        state: RemoteHostState::Connected,
        error: None,
        updated_at_ms: 0,
        connection_id: Some(Uuid::new_v4()),
        forwarding_socket: None,
    };
    let data = Store {
        hosts: BTreeMap::from([(profile.id, host)]),
    };
    std::fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
    let manager =
        RemoteHostManager::open_with_executable(path, root.path().join("must-not-execute"))
            .await
            .unwrap();
    let snapshot = manager.snapshot().await;
    assert_eq!(snapshot.hosts[0].state, RemoteHostState::Interrupted);
    assert!(snapshot.hosts[0]
        .error
        .as_deref()
        .unwrap()
        .contains("No connection was replayed"));
    assert_eq!(snapshot.hosts[0].local_url, "http://127.0.0.1:45001/");
    assert!(manager.0.running.lock().unwrap().is_empty());
}

#[tokio::test]
async fn failed_persistence_rolls_back_and_blocks_new_connections() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("hosts.json");
    let manager = RemoteHostManager::open(path.clone()).await.unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let profile = profile();
    assert!(manager
        .command(RemoteHostCommand::Save {
            profile: profile.clone()
        })
        .await
        .is_err());
    let snapshot = manager.snapshot().await;
    assert!(snapshot.hosts.is_empty());
    assert!(snapshot.error.is_some());
    assert!(manager
        .command(RemoteHostCommand::Connect { id: profile.id })
        .await
        .is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn host_authority_unlocks_only_after_the_last_owner_even_with_an_inherited_descriptor() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("hosts.json");
    let manager = RemoteHostManager::open(path.clone()).await.unwrap();
    let retained = manager.clone();
    // dup shares the same flock file description as a forked child's descriptor,
    // deterministically reproducing inheritance without forking a Tokio runtime.
    let inherited = manager.0._lock.0.try_clone().unwrap();
    drop(manager);
    assert!(RemoteHostManager::open(path.clone()).await.is_err());
    drop(retained);
    let reopened = RemoteHostManager::open(path.clone()).await.unwrap();
    drop(inherited);
    assert!(RemoteHostManager::open(path.clone()).await.is_err());
    drop(reopened);
    assert!(RemoteHostManager::open(path).await.is_ok());
}

#[cfg(unix)]
fn fixture() -> (tempfile::TempDir, PathBuf) {
    use std::os::unix::fs::PermissionsExt as _;
    let root = tempfile::tempdir().unwrap();
    let executable = root.path().join("ssh-fixture");
    std::fs::write(
        &executable,
        r#"#!/usr/bin/python3
import os,sys,json,time,subprocess
from pathlib import Path
root=Path(__file__).parent
with root.joinpath('argv.jsonl').open('a') as out:out.write(json.dumps(sys.argv[1:])+'\n')
args=sys.argv[1:]
if '-G' in args:
 print('hostname fixture')
 if args[-1]=='extra':print('localforward 0.0.0.0:1 host:2')
 sys.exit(0)
if '-O' in args:
 control=Path(args[args.index('-S')+1])
 sys.exit(0 if control.exists() else 1)
if args[-1]=='unknown':
 print('Host key verification failed.',file=sys.stderr,flush=True);sys.exit(255)
control=Path(args[args.index('-S')+1])
root.joinpath('master.pid').write_text(str(os.getpid()))
child=subprocess.Popen(['sleep','30']);root.joinpath('proxy.pid').write_text(str(child.pid))
control.write_text('private fixture socket')
while True:time.sleep(1)
"#,
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    (root, executable)
}

#[cfg(unix)]
async fn stopped(pid: &str) -> bool {
    let status = tokio::process::Command::new("ps")
        .args(["-o", "stat=", "-p", pid])
        .output()
        .await
        .unwrap();
    let state = String::from_utf8_lossy(&status.stdout);
    state.trim().is_empty() || state.trim_start().starts_with('Z')
}

#[cfg(unix)]
async fn wait_stopped(master: &str, proxy: &str) {
    tokio::time::timeout(crate::capture_client::REQUEST_TIMEOUT, async {
        while !stopped(master).await || !stopped(proxy).await {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("Owned SSH fixture processes must stop");
}

#[cfg(unix)]
#[tokio::test]
async fn connect_and_disconnect_own_the_master_and_its_proxy_descendant() {
    let (root, executable) = fixture();
    let manager =
        RemoteHostManager::open_with_executable(root.path().join("hosts.json"), executable)
            .await
            .unwrap();
    let profile = profile();
    manager
        .command(RemoteHostCommand::Save {
            profile: profile.clone(),
        })
        .await
        .unwrap();
    assert!(!root.path().join("argv.jsonl").exists());
    manager
        .command(RemoteHostCommand::Connect { id: profile.id })
        .await
        .unwrap();
    let connected = wait_state(&manager, profile.id, RemoteHostState::Connected).await;
    let socket = connected.forwarding_socket.unwrap();
    assert!(socket.ends_with("api"));
    assert!(manager
        .command(RemoteHostCommand::Connect { id: profile.id })
        .await
        .is_err());
    let master = std::fs::read_to_string(root.path().join("master.pid")).unwrap();
    let proxy = std::fs::read_to_string(root.path().join("proxy.pid")).unwrap();
    manager
        .command(RemoteHostCommand::Disconnect { id: profile.id })
        .await
        .unwrap();
    wait_state(&manager, profile.id, RemoteHostState::Disconnected).await;
    assert!(!socket.parent().unwrap().exists());
    wait_stopped(&master, &proxy).await;
    let arguments = std::fs::read_to_string(root.path().join("argv.jsonl")).unwrap();
    assert!(arguments.contains("StrictHostKeyChecking=yes"));
    assert!(arguments.contains("127.0.0.1:45001:127.0.0.1:45000"));
    drop(manager);
}

#[cfg(unix)]
#[tokio::test]
async fn alias_defined_forwards_are_refused_before_starting_a_master() {
    let (root, executable) = fixture();
    let manager =
        RemoteHostManager::open_with_executable(root.path().join("hosts.json"), executable)
            .await
            .unwrap();
    let mut profile = profile();
    profile.host = "extra".into();
    manager
        .command(RemoteHostCommand::Save {
            profile: profile.clone(),
        })
        .await
        .unwrap();
    manager
        .command(RemoteHostCommand::Connect { id: profile.id })
        .await
        .unwrap();
    let host = wait_state(&manager, profile.id, RemoteHostState::Failed).await;
    assert!(host.error.unwrap().contains("additional forwards"));
    assert!(!root.path().join("master.pid").exists());
    drop(manager);
}

#[cfg(unix)]
#[tokio::test]
async fn unknown_host_keys_are_actionable_failures_and_never_success() {
    let (root, executable) = fixture();
    let manager =
        RemoteHostManager::open_with_executable(root.path().join("hosts.json"), executable)
            .await
            .unwrap();
    let mut profile = profile();
    profile.host = "unknown".into();
    manager
        .command(RemoteHostCommand::Save {
            profile: profile.clone(),
        })
        .await
        .unwrap();
    manager
        .command(RemoteHostCommand::Connect { id: profile.id })
        .await
        .unwrap();
    let host = wait_state(&manager, profile.id, RemoteHostState::Failed).await;
    assert!(host.error.unwrap().contains("Host key verification failed"));
    drop(manager);
}

#[cfg(unix)]
#[tokio::test]
async fn cancelling_during_connection_stays_disconnected_without_an_auth_failure() {
    let (root, executable) = fixture();
    let manager =
        RemoteHostManager::open_with_executable(root.path().join("hosts.json"), executable)
            .await
            .unwrap();
    let profile = profile();
    manager
        .command(RemoteHostCommand::Save {
            profile: profile.clone(),
        })
        .await
        .unwrap();
    manager
        .command(RemoteHostCommand::Connect { id: profile.id })
        .await
        .unwrap();
    manager
        .command(RemoteHostCommand::Disconnect { id: profile.id })
        .await
        .unwrap();
    let host = wait_state(&manager, profile.id, RemoteHostState::Disconnected).await;
    assert!(host.error.is_none());
    assert!(host.connection_id.is_none());
}

#[cfg(unix)]
#[tokio::test]
async fn disconnect_still_stops_owned_processes_when_the_store_is_unwritable() {
    let (root, executable) = fixture();
    let path = root.path().join("hosts.json");
    let manager = RemoteHostManager::open_with_executable(path.clone(), executable)
        .await
        .unwrap();
    let profile = profile();
    manager
        .command(RemoteHostCommand::Save {
            profile: profile.clone(),
        })
        .await
        .unwrap();
    manager
        .command(RemoteHostCommand::Connect { id: profile.id })
        .await
        .unwrap();
    wait_state(&manager, profile.id, RemoteHostState::Connected).await;
    let master = std::fs::read_to_string(root.path().join("master.pid")).unwrap();
    let proxy = std::fs::read_to_string(root.path().join("proxy.pid")).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(manager
        .command(RemoteHostCommand::Disconnect { id: profile.id })
        .await
        .is_err());
    wait_stopped(&master, &proxy).await;
    wait_state(&manager, profile.id, RemoteHostState::Interrupted).await;
    assert!(manager.snapshot().await.error.is_some());
}
