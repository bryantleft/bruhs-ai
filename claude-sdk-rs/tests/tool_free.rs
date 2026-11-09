//! Actual isolated subprocess invocation; never uses the installed provider CLI.
#![cfg(unix)]
use claude_sdk_rs::{query, PermissionMode, QueryEvent, QueryOptions, Tools};
use std::{os::unix::fs::PermissionsExt, path::Path, time::Duration};
use tokio_stream::StreamExt;

fn fixture(directory: &Path, body: &str) {
    let path = directory.join("claude");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write owned fixture CLI");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .expect("fixture executable");
}
fn options(directory: &Path) -> QueryOptions {
    QueryOptions::new("Fixture transcript, no real user data")
        .cwd(directory)
        .tools(Tools::None)
        .permission_mode(PermissionMode::BypassPermissions)
        .env("PATH", directory.display().to_string())
}
#[tokio::test]
async fn empty_tool_inventory_disables_inherited_mcp_and_bypass_permissions() {
    let directory = tempfile::tempdir().unwrap();
    fixture(directory.path(), "printf '%s\\n' \"$@\" > arguments\nprintf '%s\\n' '{\"type\":\"result\",\"subtype\":\"success\",\"result\":\"fixture result\"}'");
    let mut stream = query(options(directory.path())).unwrap();
    let event = tokio::time::timeout(Duration::from_secs(3), stream.next())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        QueryEvent::Result {
            is_error: false,
            ..
        }
    ));
    drop(stream);
    let args = std::fs::read_to_string(directory.path().join("arguments")).unwrap();
    assert!(args.contains(
        "--tools=\n--restricted\n--strict-mcp-config\n--mcp-config={\"mcpServers\":{}}\n"
    ));
    assert!(!args.contains("--dangerously-skip-permissions"));
    assert!(!args.contains("--allowedTools"));
}
#[tokio::test]
async fn dropping_receiver_kills_and_reaps_a_silent_owned_provider_process() {
    let directory = tempfile::tempdir().unwrap();
    fixture(directory.path(), "echo $$ > owned_pid\nexec /bin/sleep 30");
    let stream = query(options(directory.path())).unwrap();
    let pid_path = directory.path().join("owned_pid");
    let pid = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(&pid_path) {
                if let Ok(pid) = text.trim().parse::<u32>() {
                    break pid;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    drop(stream);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let status = tokio::process::Command::new("/bin/kill")
                .args(["-0", &pid.to_string()])
                .stderr(std::process::Stdio::null())
                .status()
                .await
                .unwrap();
            if !status.success() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("owned silent process must be killed and reaped after receiver drop");
}
