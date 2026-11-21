use super::*;

#[cfg(unix)]
#[gpui::test]
fn cli_turn_renders_activity_and_measures_resources(cx: &mut gpui::TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    // Run only this test in a child so PATH never changes for sibling tests.
    const FIXTURE: &str = "BRUHS_TEST_THREAD_RESOURCE_FOLDER";
    let Some(folder) = std::env::var_os(FIXTURE) else {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("bin");
        let checkout = temp.path().join("checkout");
        std::fs::create_dir(&bin).unwrap();
        std::fs::create_dir(&checkout).unwrap();
        let cli = bin.join("codex");
        std::fs::write(&cli, r#"#!/bin/sh
set -eu
cat >/dev/null
cd "$BRUHS_TEST_THREAD_RESOURCE_FOLDER"
echo '{"type":"thread.started","thread_id":"fixture-thread"}'
echo '{"type":"item.started","item":{"id":"command","type":"command_execution","command":"sh job.sh"}}'
sh job.sh
echo '{"type":"item.completed","item":{"id":"command","type":"command_execution","aggregated_output":"job finished","exit_code":0}}'
echo '{"type":"item.completed","item":{"id":"patch","type":"file_change","changes":[{"kind":"add","path":"artifact.bin"}],"status":"completed"}}'
echo '{"type":"item.completed","item":{"type":"agent_message","text":"Fixture complete"}}'
echo '{"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":1}}'
"#).unwrap();
        std::fs::set_permissions(cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(checkout.join("job.sh"), "dd if=/dev/urandom of=artifact.bin bs=1024 count=256 2>/dev/null\nwhile [ ! -f finish ]; do sleep 0.1; done\n").unwrap();
        let path = std::env::join_paths(
            std::iter::once(bin).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                concat!(
                    module_path!(),
                    "::cli_turn_renders_activity_and_measures_resources"
                )
                .split_once("::")
                .unwrap()
                .1,
                "--nocapture",
            ])
            .env(FIXTURE, &checkout)
            .env("PATH", path)
            .env("CODEX_HOME", temp.path().join("codex-home"))
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    };
    let folder = std::path::PathBuf::from(folder);
    let (workspace, cx, general, code) = thread_menu_workspace(cx);
    let runtime = std::sync::Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap(),
    );
    workspace.update_in(cx, |_, _, cx| {
        cx.global_mut::<DesktopState>().runtime = runtime.clone();
    });
    cx.simulate_resize(gpui::size(px(820.), px(720.)));
    workspace.update_in(cx, |w, _, cx| {
        w.select_thread_by_id(&code, cx);
        w.active_thread().unwrap().chat.update(cx, |chat, cx| {
            chat.set_model("gpt-5.5", cx);
            chat.set_project_context(folder.to_string_lossy(), "Fixture", true, cx);
            chat.run_locked = true;
            chat.submit_prompt("Run the fixture job".into(), cx);
        });
    });
    let card = |id| {
        leaked_debug_selector(format!(
            "desktop-tool-call-card-{}",
            super::super::element_key(id)
        ))
    };
    let command_card = card("command");
    let patch_card = card("patch");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        cx.run_until_parked();
        if cx.debug_bounds(command_card).is_some() && folder.join("artifact.bin").exists() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "CLI script or running command card missing"
        );
        runtime.block_on(async { tokio::time::sleep(Duration::from_millis(25)).await });
    }
    click_debug_selector(cx, "desktop-thread-details-toggle");
    cx.run_until_parked();
    click_debug_selector(cx, "measure-thread-usage");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "detail-resource-sample");
    workspace.update_in(cx, |w, _, _| {
        let (id, _, sample) = w.details.last_measurement().unwrap();
        assert_eq!(id, &code);
        for metric in ["CPU ", "RAM ", "Read ", "Write ", "Checkout size · "] {
            assert!(sample.contains(metric), "{sample}");
        }
        let ram: f64 = sample
            .split_once("RAM ")
            .unwrap()
            .1
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(ram > 0.0, "{sample}");
        let kib: u64 = sample
            .split_once("Checkout size · ")
            .unwrap()
            .1
            .trim_end_matches(" KiB")
            .parse()
            .unwrap();
        assert!(kib >= 256, "{sample}");
    });
    click_debug_selector(cx, "desktop-thread-details-toggle");
    cx.run_until_parked();
    std::fs::write(folder.join("finish"), "").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if workspace.update_in(cx, |w, _, cx| {
            !w.active_thread().unwrap().chat.read(cx).is_streaming
        }) {
            break;
        }
        assert!(Instant::now() < deadline, "turn did not finish");
        runtime.block_on(async { tokio::time::sleep(Duration::from_millis(25)).await });
    }
    workspace.update_in(cx, |w, _, cx| {
        let chat = w.active_thread().unwrap().chat.read(cx);
        let message = chat.messages.last().unwrap();
        assert_eq!(message.content, "Fixture complete");
        assert_eq!(
            message.tool_calls[0].output.as_deref(),
            Some("job finished")
        );
        assert_eq!(
            message.tool_calls[1].output.as_deref(),
            Some("add artifact.bin")
        );
    });
    if cx.debug_bounds(patch_card).is_none() {
        click_debug_selector(cx, "desktop-work-group-1");
        cx.run_until_parked();
    }
    assert_rendered_bounds(cx, patch_card);
    click_debug_selector(cx, command_card);
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-terminal-output");
    click_debug_selector(cx, "desktop-thread-details-toggle");
    cx.run_until_parked();
    click_debug_selector(cx, "measure-thread-usage");
    cx.run_until_parked();
    workspace.update_in(cx, |w, _, cx| {
        assert!(w
            .details
            .last_measurement()
            .unwrap()
            .2
            .contains("No active local CLI"));
        w.select_thread_by_id(&general, cx);
    });
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "detail-resource-sample");
}
