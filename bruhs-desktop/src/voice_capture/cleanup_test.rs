//! Run the native dictation path against an isolated capture host and a tool-free provider.
use super::*;
use bruhs_core::dictation_cleanup::DraftContext;
use std::{
    io::{Read, Write},
    net::TcpListener,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::Command,
};

#[test]
fn cleanup_uses_bounded_draft_and_retains_raw_speech_on_provider_failure() {
    const CHILD: &str = "BRUHS_TEST_VOICE_CLEANUP_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut prefs = DictationPreferences::default();
            prefs.cleanup.enabled = true;
            prefs.cleanup.model = Some("claude-sonnet-4-6".into());
            prefs.cleanup.correction_cue = "correction".into();
            prefs.cleanup.custom = true;
            prefs.cleanup.instructions = "Use sentence case.".into();
            let draft = || DraftContext::around("prefix OMIT suffix", 7..11);
            let reply = prefs
                .transcribe(vec![1, 2], None, None, "unused", draft(), None)
                .await
                .unwrap();
            assert_eq!(reply.text, "Green apples.");
            assert!(reply.warning.is_none());
            let marker = std::env::var_os(CHILD).unwrap();
            let args = std::fs::read_to_string(Path::new(&marker).join("args")).unwrap();
            assert!(args.contains("--tools=") && args.contains("--strict-mcp-config"));
            let prompt: serde_json::Value =
                serde_json::from_str(args.lines().last().unwrap()).unwrap();
            assert_eq!(prompt["transcript"], "red correction green apples");
            assert_eq!(prompt["draft_before"], "prefix ");
            assert_eq!(prompt["draft_after"], " suffix");
            assert_eq!(prompt["editing_instructions"], "Use sentence case.");
            assert_eq!(prompt["correction_cue"], "correction");
            std::fs::write(Path::new(&marker).join("fail"), b"").unwrap();
            let reply = prefs
                .transcribe(vec![1, 2], None, None, "unused", draft(), None)
                .await
                .unwrap();
            assert_eq!(reply.text, "red correction green apples");
            assert!(reply.warning.is_some());
            std::fs::write(Path::new(&marker).join("wait"), b"").unwrap();
            let (started, ready) = tokio::sync::oneshot::channel();
            let transcription =
                prefs.transcribe(vec![1, 2], None, None, "unused", draft(), Some(started));
            let (reply, ()) = tokio::join!(transcription, async {
                ready.await.unwrap().send(()).unwrap();
            });
            let reply = reply.unwrap();
            assert_eq!(reply.text, "red correction green apples");
            assert!(reply.warning.is_none());
        });
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let cli = root.path().join("claude");
    std::fs::write(&cli, r#"#!/bin/sh
printf '%s\n' "$@" > "$BRUHS_TEST_VOICE_CLEANUP_CHILD/args"
if test -f "$BRUHS_TEST_VOICE_CLEANUP_CHILD/wait"; then exec /bin/sleep 10; fi
if test -f "$BRUHS_TEST_VOICE_CLEANUP_CHILD/fail"; then exit 1; fi
printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"result":"{\"text\":\"Green apples.\"}"}'
"#).unwrap();
    std::fs::set_permissions(cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    let server = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let url = format!("http://{}", server.local_addr().unwrap());
    let token = root.path().join("token");
    std::fs::write(&token, "a".repeat(64)).unwrap();
    let host = std::thread::spawn(move || {
        for mut socket in server.incoming().flatten().take(3) {
            let mut bytes = Vec::new();
            let mut part = [0; 512];
            while !bytes.windows(4).any(|b| b == b"\r\n\r\n") {
                let n = socket.read(&mut part).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&part[..n]);
            }
            assert!(bytes.starts_with(b"POST /dictation?"));
            let json = r#"{"text":"red correction green apples","model":"tiny_en"}"#;
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}",json.len()).unwrap();
        }
    });
    let result=Command::new(std::env::current_exe().unwrap())
        .arg("--exact").arg("voice_capture::cleanup_test::cleanup_uses_bounded_draft_and_retains_raw_speech_on_provider_failure")
        .arg("--nocapture").env_clear().env(CHILD,root.path()).env("PATH",root.path()).env("HOME",root.path())
        .env("BRUHS_CAPTURE_URL",&url).env("BRUHS_CAPTURE_TOKEN_FILE",&token)
        .env("BRUHS_SERVER_URL","http://127.0.0.1:0").output().unwrap();
    assert!(
        result.status.success(),
        "{} {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    host.join().unwrap();
}
