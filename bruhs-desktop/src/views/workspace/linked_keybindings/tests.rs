#![expect(clippy::unwrap_used, reason = "test fixture helpers")]

use super::*;
use ::core::prelude::v1::test;
use bruhs_models::remote_hosts::{RemoteHost, RemoteHostProfile, RemoteHostState};
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::unix::net::UnixListener,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

type ThreadUploads = Arc<std::sync::Mutex<Vec<(String, String, Vec<u8>)>>>;

pub(super) struct HostFixture {
    pub(super) host: RemoteHost,
    pub(super) report_deleted: Arc<AtomicBool>,
    pub(super) thread_attempts: Arc<std::sync::Mutex<Vec<(String, serde_json::Value)>>>,
    pub(super) thread_uploads: ThreadUploads,
    pub(super) run_attempts: Arc<std::sync::Mutex<Vec<String>>>,
    pub(super) schedule_attempts: Arc<std::sync::atomic::AtomicUsize>,
    directory: tempfile::TempDir,
    deny: Arc<AtomicBool>,
    pub(super) revoked: Arc<AtomicBool>,
    opened: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl HostFixture {
    pub(super) fn new(name: &str, key: &str) -> Self {
        let directory = tempfile::tempdir_in("/tmp").unwrap();
        let state = directory.path().join("state.json");
        let socket = directory.path().join("api");
        let listener = UnixListener::bind(&socket).unwrap();
        let original = Keybinding {
            key: key.into(),
            command: "branch.menu".into(),
            when: None,
        };
        std::fs::write(
            &state,
            serde_json::to_vec(
                &serde_json::json!({"keybinding_schema":1,"keybindings":[original]}),
            )
            .unwrap(),
        )
        .unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let deny = Arc::new(AtomicBool::new(false));
        let denied = deny.clone();
        let revoked = Arc::new(AtomicBool::new(false));
        let revocation = revoked.clone();
        let opened = Arc::new(AtomicBool::new(false));
        let open = opened.clone();
        let schedule_attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let attempts = schedule_attempts.clone();
        let run_attempts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let runs = run_attempts.clone();
        let thread_attempts = Arc::new(std::sync::Mutex::new(
            Vec::<(String, serde_json::Value)>::new(),
        ));
        let threads = thread_attempts.clone();
        let thread_uploads = Arc::new(std::sync::Mutex::new(
            Vec::<(String, String, Vec<u8>)>::new(),
        ));
        let uploads = thread_uploads.clone();
        let report_deleted = Arc::new(AtomicBool::new(false));
        let deleted = report_deleted.clone();
        let delivery_id = format!("{}-request", name.to_lowercase());
        let worker = std::thread::spawn(move || {
            let mut delivery_reads = 0;
            let mut report = serde_json::json!({"id":"report","title":"Weekly report","project_path":"/srv/reports","enabled":false,"schedule":{"type":"webhook"},"next_run_at":null});

            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                if stopped.load(Ordering::SeqCst) {
                    break;
                }
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut reader = BufReader::new(&stream);
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                let mut length = 0;
                let mut token = String::new();
                for line in (&mut reader).lines() {
                    let line = line.unwrap();
                    if line.is_empty() {
                        break;
                    }
                    if let Some((k, v)) = line.split_once(':') {
                        match k.to_ascii_lowercase().as_str() {
                            "content-length" => length = v.trim().parse().unwrap(),
                            "authorization" => token = v.trim().into(),
                            _ => {}
                        }
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let start_threads = token == format!("Bearer {}", "b".repeat(64));
                let editable = token == format!("Bearer {}", "a".repeat(64));
                let toggle = token == format!("Bearer {}", "f".repeat(64));
                let run = token == format!("Bearer {}", "d".repeat(64));
                let delete = token == format!("Bearer {}", "9".repeat(64));
                let deliveries = token == format!("Bearer {}", "8".repeat(64));
                let schedules = deliveries
                    || delete
                    || run
                    || toggle
                    || token == format!("Bearer {}", "e".repeat(64));
                let (status, body) = if revocation.load(Ordering::SeqCst)
                    || (first.starts_with("POST")
                        && !first.starts_with("POST /api/projects/threads/")
                        && (!editable || denied.load(Ordering::SeqCst)))
                {
                    ("403 Forbidden", Vec::new())
                } else if start_threads && first.starts_with("POST /api/projects/threads/") {
                    let target = first.split_whitespace().nth(1).unwrap();
                    let (path, query) = target.split_once('?').unwrap_or((target, ""));
                    let parts: Vec<_> = path.split('/').collect();
                    if parts.len() == 6 && parts[5] == "attachments" {
                        let id = parts[4].to_owned();
                        let name = query
                            .split('&')
                            .find_map(|pair| pair.strip_prefix("name="))
                            .unwrap_or("file")
                            .to_owned();
                        let bytes = body.len() as u64;
                        uploads
                            .lock()
                            .unwrap()
                            .push((id.clone(), name.clone(), body));
                        (
                            "200 OK",
                            serde_json::to_vec(&serde_json::json!({
                                "id": format!("linked-thread-fixture-{name}"),
                                "name": name,
                                "bytes": bytes,
                            }))
                            .unwrap(),
                        )
                    } else {
                        ("404 Not Found", Vec::new())
                    }
                } else if first.starts_with("GET /api/projects ") {
                    if start_threads || schedules || token == format!("Bearer {}", "c".repeat(64)) {
                        ("200 OK", serde_json::to_vec(&serde_json::json!([{
                            "name":"Build editor", "title":"Build editor", "path":"/srv/editor",
                            "identity":{"repository":"remote:example.org/app", "relative_path":"", "label":"example.org/app"}
                        }])).unwrap())
                    } else {
                        ("403 Forbidden", Vec::new())
                    }
                } else if start_threads && first.starts_with("GET /api/projects/thread-checkout?") {
                    (
                        "200 OK",
                        serde_json::to_vec(&serde_json::json!({
                            "path":"/srv/editor","branch":"host-branch","model":"opencode/default"
                        }))
                        .unwrap(),
                    )
                } else if start_threads && first.starts_with("PUT /api/projects/threads/") {
                    let id = first
                        .split_whitespace()
                        .nth(1)
                        .unwrap()
                        .rsplit('/')
                        .next()
                        .unwrap();
                    let mut requests = threads.lock().unwrap();
                    requests.push((id.into(), serde_json::from_slice(&body).unwrap()));
                    if requests.len() == 1 {
                        // Model a first send that succeeded on the host before the response was lost.
                        ("503 Service Unavailable", Vec::new())
                    } else {
                        ("200 OK", serde_json::to_vec(id).unwrap())
                    }
                } else if start_threads && first.starts_with("GET /api/projects/threads ") {
                    let requests = threads.lock().unwrap();
                    let mut seen = BTreeSet::new();
                    let rows: Vec<_> = requests.iter().filter(|(id, _)| seen.insert(id)).map(|(id, start)|
                        serde_json::json!({"id":id,"title":"Host conversation","checkout":start["checkout"],"updated_at":1700000000000_i64})
                    ).collect();
                    drop(requests);
                    ("200 OK", serde_json::to_vec(&rows).unwrap())
                } else if start_threads && first.starts_with("GET /api/projects/threads/") {
                    let id = first
                        .split_whitespace()
                        .nth(1)
                        .unwrap()
                        .rsplit('/')
                        .next()
                        .unwrap();
                    let requests = threads.lock().unwrap();
                    if let Some((_, start)) = requests.iter().find(|(key, _)| key == id) {
                        ("200 OK", serde_json::to_vec(&serde_json::json!({
                            "id":id,"title":"Host conversation","checkout":start["checkout"],
                            "state":"completed","messages":[{"role":"user","text":start["message"],"at":1700000000000_i64}],
                            "live_output":"","error":null
                        })).unwrap())
                    } else {
                        ("404 Not Found", Vec::new())
                    }
                } else if first.starts_with("GET /api/projects/scheduled-tasks ") && schedules {
                    let daily = serde_json::json!({"id":"daily","title":"Daily review","project_path":"/srv/editor","enabled":true,"schedule":{"type":"calendar","times":["09:00"],"weekdays":[],"time_zone":"UTC"},"next_run_at":null});
                    let mut tasks = vec![daily];
                    if !deleted.load(Ordering::SeqCst) {
                        tasks.push(report.clone());
                    }
                    ("200 OK", serde_json::to_vec(&tasks).unwrap())
                } else if first.starts_with("GET /api/projects/scheduled-tasks/report/deliveries") {
                    let summary = serde_json::json!({"id":delivery_id,"received_at":1700000000000_i64,"outcome":"filtered"});
                    if !deliveries {
                        ("403 Forbidden", Vec::new())
                    } else if first
                        .starts_with("GET /api/projects/scheduled-tasks/report/deliveries ")
                    {
                        delivery_reads += 1;
                        let rows = if delivery_reads == 1 {
                            vec![]
                        } else {
                            vec![summary]
                        };
                        ("200 OK", serde_json::to_vec(&rows).unwrap())
                    } else if first.starts_with(&format!(
                        "GET /api/projects/scheduled-tasks/report/deliveries/{delivery_id} "
                    )) {
                        ("200 OK",serde_json::to_vec(&serde_json::json!({"summary":summary,"body":delivery_id,"headers":"x-source: fixture","query":"","body_truncated":false})).unwrap())
                    } else {
                        ("404 Not Found", Vec::new())
                    }
                } else if first.starts_with("DELETE /api/projects/scheduled-tasks/report ") {
                    if delete {
                        deleted.store(true, Ordering::SeqCst);
                        ("204 No Content", Vec::new())
                    } else {
                        ("403 Forbidden", Vec::new())
                    }
                } else if let Some(path) =
                    first.strip_prefix("PUT /api/projects/scheduled-tasks/daily/runs/")
                {
                    let mut runs = runs.lock().unwrap();
                    let id = path.split_whitespace().next().unwrap();
                    uuid::Uuid::parse_str(id).unwrap();
                    runs.push(id.to_owned());
                    if !run {
                        ("403 Forbidden", Vec::new())
                    } else if runs.len() == 1 {
                        ("503 Service Unavailable", Vec::new())
                    } else {
                        ("204 No Content", Vec::new())
                    }
                } else if first.starts_with("PUT /api/projects/scheduled-tasks/report/enabled ") {
                    let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                    if !toggle {
                        ("403 Forbidden", Vec::new())
                    } else if attempt == 0 {
                        (
                            "409 Conflict",
                            br#"{"error":"Another automation would also start."}"#.to_vec(),
                        )
                    } else {
                        report["enabled"] = serde_json::from_slice::<serde_json::Value>(&body)
                            .unwrap()["enabled"]
                            .clone();
                        ("200 OK", serde_json::to_vec(&report).unwrap())
                    }
                } else if first.starts_with("POST /api/keybindings/open ") {
                    open.store(true, Ordering::SeqCst);
                    ("204 No Content", Vec::new())
                } else {
                    assert!(first.contains(" /api/keybindings "));
                    let edit = (!body.is_empty()).then(|| serde_json::from_slice(&body).unwrap());
                    let rules =
                        bruhs_core::keybinding_file::environment_rules(&state, edit.as_ref())
                            .unwrap();
                    (
                        "200 OK",
                        serde_json::to_vec(&EnvironmentKeybindings { editable, rules }).unwrap(),
                    )
                };
                write!(stream,"HTTP/1.1 {status}\r\nx-bruhs-start-project-threads: {start_threads}\r\nx-bruhs-read-scheduled-tasks: {schedules}\r\nx-bruhs-toggle-scheduled-tasks: {toggle}\r\nx-bruhs-run-scheduled-tasks: {run}\r\nx-bruhs-delete-scheduled-tasks: {delete}\r\nx-bruhs-webhook-deliveries: {deliveries}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",body.len()).unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        Self {
            report_deleted,
            thread_attempts,
            thread_uploads,
            run_attempts,
            schedule_attempts,
            host: RemoteHost {
                profile: RemoteHostProfile {
                    id: uuid::Uuid::new_v4(),
                    name: name.into(),
                    host: name.into(),
                    ssh_port: 22,
                    local_port: 45001,
                    remote_port: 45000,
                    connect_timeout_seconds: 8,
                },
                state: RemoteHostState::Connected,
                error: None,
                updated_at_ms: 0,
                local_url: String::new(),
                connection_id: Some(uuid::Uuid::new_v4()),
                forwarding_socket: Some(socket),
            },
            directory,
            deny,
            revoked,
            opened,
            stop,
            worker: Some(worker),
        }
    }
    fn state(&self) -> PathBuf {
        self.directory.path().join("state.json")
    }
}
impl Drop for HostFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ =
            std::os::unix::net::UnixStream::connect(self.host.forwarding_socket.as_ref().unwrap());
        self.worker.take().unwrap().join().unwrap();
    }
}
#[gpui::test]
fn linked_keybindings_select_the_host_and_retry_a_partial_save(cx: &mut gpui::TestAppContext) {
    cx.executor().allow_parking();
    let first = HostFixture::new("Dev", "alt+8");
    let second = HostFixture::new("Build", "alt+9");
    let (ws, cx, _, _) = super::super::tests::thread_menu_workspace(cx);
    cx.simulate_resize(gpui::size(px(1040.), px(820.)));
    ws.update_in(cx, |ws, w, cx| {
        ws.open_settings(&OpenSettings, w, cx);
        ws.set_settings_section(SettingsSection::Environments, cx);
    });
    let click = |cx: &mut gpui::VisualTestContext, id: &'static str| {
        let bounds = cx
            .debug_bounds(id)
            .unwrap_or_else(|| panic!("{id} missing"));
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        cx.run_until_parked();
    };
    let connect = |host: &HostFixture, letter: &str, cx: &mut gpui::VisualTestContext| {
        ws.update_in(cx, |ws, w, cx| {
            ws.set_settings_section(SettingsSection::Environments, cx);
            ws.open_shortcut_link(Some(&host.host), w, cx);
            ws.linked_keybindings
                .form
                .as_ref()
                .unwrap()
                .token
                .update(cx, |f, cx| f.set_value(letter.repeat(64), w, cx));
        });
        cx.run_until_parked();
        click(cx, "connect-shortcuts");
        ws.read_with(cx, |ws, _| {
            assert!(
                ws.linked_keybindings.error.is_none(),
                "{:?}",
                ws.linked_keybindings.error
            )
        });
    };
    connect(&first, "a", cx);
    connect(&second, "b", cx);
    ws.update_in(cx, |ws, _, cx| {
        ws.view_host_shortcuts(Some(1), cx);
        assert!(ws.settings_keybindings().iter().any(|r| r.key == "alt+9"));
        assert!(!ws.linked_keybindings.editable());
        assert_eq!(ws.remote_file_button_state(), Some(false));
    });
    cx.run_until_parked();
    click(cx, "desktop-keybindings-add");
    ws.read_with(cx, |ws, _| assert!(ws.keybinding_draft.is_none()));
    connect(&second, "a", cx);
    ws.update_in(cx, |ws, _, cx| ws.view_host_shortcuts(Some(1), cx));
    cx.run_until_parked();
    let new = Keybinding {
        command: "branch.menu".into(),
        key: "alt+shift+8".into(),
        when: None,
    };
    second.deny.store(true, Ordering::SeqCst);
    ws.update_in(cx, |ws, _, cx| {
        ws.apply_binding_edit(
            KeybindingEdit::Upsert {
                rule: new.clone(),
                replace: None,
            },
            cx,
        );
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert!(ws.linked_keybindings.retry.is_some());
        assert!(!ws.keybindings.contains(&new));
    });
    assert!(
        bruhs_core::keybinding_file::environment_rules(&first.state(), None)
            .unwrap()
            .contains(&new)
    );
    // The first host accepted the edit. Reject further writes there to prove
    // Retry sends only the operations still outstanding.
    first.deny.store(true, Ordering::SeqCst);
    second.deny.store(false, Ordering::SeqCst);
    click(cx, "retry-shortcut-sync");
    for (host, key) in [(&first, "alt+8"), (&second, "alt+9")] {
        let rules = bruhs_core::keybinding_file::environment_rules(&host.state(), None).unwrap();
        assert_eq!(rules.iter().filter(|r| *r == &new).count(), 1);
        assert!(rules.iter().any(|r| r.key == key));
    }
    ws.read_with(cx, |ws, _| {
        assert!(ws.keybindings.contains(&new));
        assert!(ws.settings_keybindings().contains(&new));
    });
    click(cx, "settings-keymap-file");
    assert!(second.opened.load(Ordering::SeqCst));
    assert!(!first.opened.load(Ordering::SeqCst));
}

#[gpui::test]
async fn linked_keybindings_restore_and_refresh_from_the_selected_host(
    cx: &mut gpui::TestAppContext,
) {
    use super::super::tests::launch;
    launch::init(cx);
    let clock = cx.executor();
    let mut next_launch = cx.clone();
    let state_dir = tempfile::tempdir().unwrap();
    let path = state_dir.path().join("desktop-state.json");
    let host = HostFixture::new("Development", "alt+8");
    let token = "a".repeat(64);
    let (ws, win) = launch::launch(cx, &path);
    ws.update_in(win, |ws, w, cx| {
        ws.open_settings(&OpenSettings, w, cx);
        ws.open_shortcut_link(Some(&host.host), w, cx);
        ws.linked_keybindings
            .form
            .as_ref()
            .unwrap()
            .token
            .update(cx, |input, cx| input.set_value(token.clone(), w, cx));
        ws.connect_shortcut_host(cx);
    });
    win.run_until_parked();
    launch::saved(&ws, win).await;
    let metadata = std::fs::read_to_string(&path).unwrap();
    assert!(metadata.contains("shortcut_links") && metadata.contains("Development"));
    assert!(!metadata.contains(&token));
    assert!(!metadata.contains(
        host.host
            .forwarding_socket
            .as_ref()
            .unwrap()
            .to_str()
            .unwrap()
    ));
    let vault = ws.read_with(win, |ws, _| ws.linked_keybindings.vault.clone());
    launch::quit(&ws, win);

    let (reopened, win) = launch::launch(&mut next_launch, &path);
    win.simulate_resize(gpui::size(px(820.), px(640.)));
    let snapshot = |h: RemoteHost| RemoteHostSnapshot {
        hosts: vec![h],
        error: None,
    };
    let rule = Keybinding {
        command: "branch.menu".into(),
        key: "alt+shift+8".into(),
        when: None,
    };
    reopened.update_in(win, |ws, w, cx| {
        let state = &mut ws.linked_keybindings;
        state.vault = vault.clone();
        assert_eq!(state.selected, Some(0));
        let saved = &state.hosts[0];
        assert!(saved.token.is_empty() && !saved.data.editable);
        assert!(saved.tunnel.as_ref().unwrap().socket.is_none());
        let mut changed = host.host.clone();
        changed.profile.remote_port += 1;
        state.update_tunnels(&snapshot(changed.clone()));
        assert!(state.hosts[0].tunnel.as_ref().unwrap().socket.is_none());
        assert!(vault
            .read(
                state.hosts[0].credential,
                &Destination::from_profile(&changed.profile)
            )
            .is_err());
        state.update_tunnels(&snapshot(host.host.clone()));
        ws.open_settings(&OpenSettings, w, cx);
        ws.set_settings_section(SettingsSection::Keyboard, cx);
    });
    win.run_until_parked();
    reopened.update_in(win, |ws, w, cx| {
        assert!(
            ws.linked_keybindings.editable(),
            "{:?}",
            ws.linked_keybindings.hosts[0].problem
        );
        let index = ws
            .settings_keybindings()
            .iter()
            .position(|r| r.key == "alt+8")
            .unwrap();
        ws.select_keybinding(index, w, cx);
        ws.keybinding_key_input
            .update(cx, |input, cx| input.set_value("alt+7", w, cx));
    });
    bruhs_core::keybinding_file::environment_rules(
        &host.state(),
        Some(&KeybindingEdit::Upsert {
            rule: rule.clone(),
            replace: None,
        }),
    )
    .unwrap();
    clock.advance_clock(Duration::from_secs(2));
    win.run_until_parked();
    reopened.read_with(win, |ws, cx| {
        assert!(ws.settings_keybindings().contains(&rule));
        assert!(ws.keybinding_edit_open);
        assert_eq!(ws.keybinding_key_input.read(cx).value().as_ref(), "alt+7");
        assert!(!ws.keybindings.contains(&rule));
    });
    host.revoked.store(true, Ordering::SeqCst);
    clock.advance_clock(Duration::from_secs(2));
    win.run_until_parked();
    reopened.update_in(win, |ws, _w, cx| {
        assert!(!ws.linked_keybindings.editable() && !ws.keybinding_edit_open);
        assert!(ws.settings_keybindings().is_empty());
        assert!(ws.linked_keybindings.hosts[0]
            .problem
            .as_ref()
            .unwrap()
            .contains("denied shortcut access"));
        ws.view_host_shortcuts(None, cx);
        assert!(ws.linked_keybindings.editable());
        let local_rule = Keybinding {
            command: "branch.menu".into(),
            key: "alt+shift+6".into(),
            when: None,
        };
        ws.apply_binding_edit(
            KeybindingEdit::Upsert {
                rule: local_rule.clone(),
                replace: None,
            },
            cx,
        );
        assert!(ws.keybindings.contains(&local_rule));
        assert!(
            !bruhs_core::keybinding_file::environment_rules(&host.state(), None)
                .unwrap()
                .contains(&local_rule)
        );
        ws.view_host_shortcuts(Some(0), cx);
        host.revoked.store(false, Ordering::SeqCst);
        ws.linked_keybindings
            .update_tunnels(&snapshot(host.host.clone()));
    });
    clock.advance_clock(Duration::from_secs(2));
    win.run_until_parked();
    let id = reopened.update_in(win, |ws, _w, cx| {
        assert!(ws.linked_keybindings.editable());
        let h = ws.linked_keybindings.hosts[0].clone();
        ws.disconnect_shortcut_host(&h, cx);
        h.credential
    });
    win.run_until_parked();
    launch::saved(&reopened, win).await;
    assert!(!std::fs::read_to_string(&path)
        .unwrap()
        .contains("shortcut_links"));
    assert!(vault
        .read(id, &Destination::from_profile(&host.host.profile))
        .is_err());
}
