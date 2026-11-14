//! Real MCP executable → stdio → private socket → native foreground reader.
use super::super::super::DesktopState;
use super::{connect, fixture, large_ledger, WorkspaceBridge};
use bruhs_models::{AgentAccess, AgentDomain};
use gpui::TestAppContext;

#[gpui::test]
#[ignore = "manual optimized full stdio timing; set BRUHS_PERF_MCP_BIN and BRUHS_PERF_STDIO_OUTPUT, then run life_read_native_stdio_cost --ignored --nocapture"]
async fn life_read_native_stdio_cost(cx: &mut TestAppContext) {
    let binary = std::env::var_os("BRUHS_PERF_MCP_BIN").expect("optimized MCP executable");
    let output = std::env::var_os("BRUHS_PERF_STDIO_OUTPUT").expect("measurement output path");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let harness = root.join("scripts/performance-mcp-stdio.py");
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, _| {
        w.life_context = large_ledger();
        w.automation_state
            .permissions
            .set(AgentDomain::Finance, AgentAccess::Read, true);
    });
    connect(&workspace, cx).await;
    let receiver = workspace.update_in(cx, |w, _, cx| {
        let super::super::ConnectionState::Connected { session, .. } = &w.mcp_bridge.state else {
            panic!("bridge connected")
        };
        let connection = session.connection_path().to_owned();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        cx.global::<DesktopState>().runtime.spawn(async move {
            let result = tokio::process::Command::new("python3")
                .arg(harness)
                .arg("--binary")
                .arg(binary)
                .arg("--output")
                .arg(output)
                .arg("--native-connection")
                .arg(connection)
                .arg("--startup-samples")
                .arg("100")
                .arg("--samples")
                .arg("200")
                .kill_on_drop(true)
                .output()
                .await;
            let _ = sender.send(result);
        });
        receiver
    });
    let result = receiver.await.unwrap().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    eprintln!("{}", String::from_utf8_lossy(&result.stdout));
    workspace.update_in(cx, |w, _, _| {
        assert_eq!(w.life_context.finance.transactions.len(), 100_001);
        assert!(w.automation_state.workflows.is_empty());
        w.mcp_bridge = WorkspaceBridge::default();
    });
    // The harness validates complete pages, IDs, redaction and catalog mode.
    // The existing socket fixture independently checks grant revocation.
}
