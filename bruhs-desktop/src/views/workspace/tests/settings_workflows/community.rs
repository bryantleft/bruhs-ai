use super::super::launch::{init, launch, quit, saved};
use super::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Cursor, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{
    atomic::{AtomicBool, AtomicU8, Ordering},
    Arc, Mutex,
};

struct RegistryFixture {
    address: SocketAddr,
    revision: Arc<AtomicU8>,
    bad_checksum: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<String>>>,
    worker: Option<std::thread::JoinHandle<()>>,
}
fn manifest(revision: u8) -> serde_json::Value {
    let mut themes = vec![
        json!({"label":"Harbor Dark","uiTheme":"vs-dark","path":"./themes/dark.json"}),
        json!({"label":"Harbor Light","uiTheme":"vs","path":"./themes/light.json"}),
    ];
    if revision == 1 {
        themes.push(json!({"label":"Coast","uiTheme":"vs-dark","path":"./coast.json"}));
    }
    json!({"name":"harbor","publisher":"studio","license":"MIT","version":revision.to_string(),"contributes":{"themes":themes}})
}
fn archive(revision: u8) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (path,body) in [
        ("extension/package.json",manifest(revision).to_string()),
        ("extension/themes/base.json",r##"{/* inherited */ "colors":{"editor.background":"#102030","editor.foreground":"#eeeeee"},}"##.into()),
        ("extension/themes/dark.json",json!({"include":"base.json","colors":{"focusBorder":"#3399ff"}}).to_string()),
        ("extension/themes/light.json",json!({"include":"base.json","colors":{"editor.background":if revision==1{"#fafafa"}else{"#fefdfc"}}}).to_string()),
        ("extension/coast.json",json!({"include":"./themes/base.json"}).to_string()),
    ]{zip.start_file(path,options).unwrap();zip.write_all(body.as_bytes()).unwrap();}
    zip.finish().unwrap().into_inner()
}
impl RegistryFixture {
    fn new() -> Self {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let revision = Arc::new(AtomicU8::new(1));
        let current = revision.clone();
        let bad_checksum = Arc::new(AtomicBool::new(false));
        let corrupt = bad_checksum.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let finished = stop.clone();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = requests.clone();
        let worker = std::thread::spawn(move || {
            let origin = format!("http://{address}");
            let archives = [archive(1), archive(2)];
            for stream in server.incoming() {
                let mut stream = stream.unwrap();
                if finished.load(Ordering::Relaxed) {
                    break;
                }
                let mut first = String::new();
                let mut reader = BufReader::new(&stream);
                reader.read_line(&mut first).unwrap();
                for line in reader.lines() {
                    if line.unwrap().is_empty() {
                        break;
                    }
                }
                let mut fields = first.split_ascii_whitespace();
                let method = fields.next().unwrap();
                let path = fields.next().unwrap();
                log.lock().unwrap().push(path.to_owned());
                let revision = current.load(Ordering::Relaxed);
                let package = &archives[usize::from(revision - 1)];
                let data=match path.split('?').next().unwrap(){
                    "/api/-/search"=>json!({"extensions":[{"namespace":"studio","name":"harbor"},{"namespace":"studio","name":"private"}]}).to_string().into_bytes(),
                    "/api/studio/harbor"|"/api/studio/private"=>json!({"namespace":"studio","name":"harbor","displayName":"Harbor","version":revision.to_string(),"license":if path.ends_with("private"){"Commercial"}else{"MIT"},"files":{"manifest":format!("{origin}/manifest"),"download":format!("{origin}/package"),"sha256":format!("{origin}/checksum")}}).to_string().into_bytes(),
                    "/manifest"=>manifest(revision).to_string().into_bytes(),
                    "/package"=>package.clone(),
                    "/checksum"=>if corrupt.load(Ordering::Relaxed){"0".repeat(64)}else{format!("{:x}",Sha256::digest(package))}.into_bytes(),
                    _=>Vec::new()
                };
                write!(
                    &mut stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    data.len()
                )
                .unwrap();
                if method != "HEAD" {
                    let _ = stream.write_all(&data);
                }
            }
        });
        Self {
            address,
            revision,
            bad_checksum,
            stop,
            requests,
            worker: Some(worker),
        }
    }
}
impl Drop for RegistryFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(self.address);
        if let Some(w) = self.worker.take() {
            w.join().unwrap();
        }
    }
}

#[gpui::test]
async fn community_theme_search_install_update_and_restart(cx: &mut gpui::TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state.json");
    let registry = RegistryFixture::new();
    let origin = format!("http://{}", registry.address);
    let mut relaunch = cx.clone();
    let (workspace, cx) = launch(cx, &state);
    cx.simulate_resize(gpui::size(px(1200.), px(1000.)));
    workspace.update_in(cx, |ws, w, cx| {
        ws.open_settings(&OpenSettings, w, cx);
        ws.set_settings_section(super::super::super::SettingsSection::Appearance, cx);
    });
    let open = |cx: &mut gpui::VisualTestContext| {
        click_debug_selector(cx, "settings-add-theme");
        workspace.update(cx, |ws, cx| ws.community_registry_for_test(&origin, cx));
        click_debug_selector(cx, "community-popular-Dracula");
        cx.run_until_parked();
    };
    open(cx);
    click_debug_selector(cx, "community-sort");
    assert!(
        cx.debug_bounds("community-sort-rating").is_some(),
        "opened sort menu"
    );
    super::super::keys::press(cx, "escape");
    assert!(cx.debug_bounds("community-sort-rating").is_none());
    assert!(
        cx.debug_bounds("community-search").is_some(),
        "Escape keeps Add a theme open"
    );
    click_debug_selector(cx, "community-sort");
    super::super::keys::press(cx, "down");
    super::super::keys::press(cx, "enter");
    cx.run_until_parked();
    assert!(registry
        .requests
        .lock()
        .unwrap()
        .iter()
        .any(|p| p.contains("sortBy=rating")));
    registry.bad_checksum.store(true, Ordering::Relaxed);
    click_debug_selector(cx, "community-install-studio.harbor");
    cx.run_until_parked();
    assert!(cx.debug_bounds("community-theme-error").is_some());
    workspace.read_with(cx, |ws, _| assert!(ws.custom_themes.empty()));
    registry.bad_checksum.store(false, Ordering::Relaxed);
    click_debug_selector(cx, "community-install-studio.harbor");
    cx.run_until_parked();
    let first = workspace.read_with(cx, |ws, _| serde_json::to_value(&ws.custom_themes).unwrap());
    let themes = first["themes"].as_array().unwrap();
    assert_eq!(themes.len(), 2);
    let pair = &themes[0];
    assert_eq!(pair["colors"]["canvas"], "#fafafa");
    assert_eq!(pair["variants"]["dark"]["canvas"], "#102030");
    assert_eq!(pair["collection"]["id"], "open-vsx:studio.harbor");
    registry.revision.store(2, Ordering::Relaxed);
    open(cx);
    click_debug_selector(cx, "community-install-studio.harbor");
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(serde_json::to_value(&ws.custom_themes).unwrap(), first)
    });
    click_debug_selector(cx, "community-install-studio.harbor");
    cx.simulate_prompt_answer("Update theme");
    cx.run_until_parked();
    saved(&workspace, cx).await;
    quit(&workspace, cx);
    let (workspace, cx) = launch(&mut relaunch, &state);
    workspace.read_with(cx, |ws, _| {
        let lib = serde_json::to_value(&ws.custom_themes).unwrap();
        assert_eq!(lib["themes"].as_array().unwrap().len(), 1);
        assert_eq!(lib["themes"][0]["id"], pair["id"]);
        assert_eq!(lib["themes"][0]["colors"]["canvas"], "#fefdfc");
        assert!(lib["dark"].is_null() && lib["light"].is_null());
    });
}

#[gpui::test]
async fn settings_collection_swatches_actions_and_variant_removal(cx: &mut gpui::TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state.json");
    let mut relaunch = cx.clone();
    let (workspace, cx) = launch(cx, &state);
    let library = json!({"themes":[
        {"version":1,"id":"pair","name":"Tide Coast","appearance":"light","colors":{"canvas":"#fafafa"},"variants":{"dark":{"canvas":"#112233"}},"collection":{"id":"studio.tide","label":"Tide Collection"}},
        {"version":1,"id":"deep","name":"Tide Deep","appearance":"dark","colors":{"canvas":"#202038"},"collection":{"id":"studio.tide","label":"Tide Collection"}},
        {"version":1,"id":"midnight","name":"Tide Midnight","appearance":"dark","colors":{"canvas":"#030405"},"collection":{"id":"studio.tide","label":"Tide Collection"}},
        {"version":1,"id":"custom-sand","name":"Sand","appearance":"light","colors":{"canvas":"#eeeeee"}}
    ]});
    cx.simulate_resize(gpui::size(px(1440.), px(1100.)));
    workspace.update_in(cx, |ws, w, cx| {
        ws.custom_themes = Arc::new(serde_json::from_value(library).unwrap());
        ws.open_settings(&OpenSettings, w, cx);
        ws.set_settings_section(super::super::super::SettingsSection::Appearance, cx);
        ws.settings_scroll
            .set_offset(gpui::point(px(0.), px(-420.)));
        cx.notify();
    });
    let library = |cx: &mut gpui::VisualTestContext| {
        workspace.read_with(cx, |ws, _| serde_json::to_value(&ws.custom_themes).unwrap())
    };
    const CARD: &str = "settings-theme-collection-studio.tide";
    const FAN: &str = "theme-variants-studio.tide-dark-current";
    const OPTION: &str = "theme-variants-studio.tide-dark-deep";
    const REMOVE: &str = "theme-remove-collection-studio.tide";
    let center = rendered_bounds(cx, CARD).center();
    workspace.update(cx, |ws, cx| {
        let offset = ws.settings_scroll.offset();
        ws.settings_scroll
            .set_offset(gpui::point(offset.x, offset.y + px(460.) - center.y));
        cx.notify();
    });
    click_debug_selector(cx, CARD);
    assert_eq!(library(cx)["dark"], "pair");
    assert_eq!(library(cx)["light"], "pair");
    assert!(cx.debug_bounds("settings-theme-custom-deep").is_none());
    click_debug_selector(cx, FAN);
    let trigger = super::super::keys::focused(cx).expect("dark preview takes focus");
    click_debug_selector(cx, "theme-variants-studio.tide-light-current");
    super::super::keys::tab_to(cx, &trigger, "dark variants");
    cx.run_until_parked();
    assert!(
        cx.debug_bounds(OPTION).is_some(),
        "keyboard focus reveals alternate swatches"
    );
    cx.simulate_keystrokes("tab tab tab");
    super::super::keys::press(cx, "enter");
    assert_eq!(library(cx)["dark"], "midnight");
    let primary = rendered_bounds(cx, FAN);
    cx.simulate_mouse_move(primary.center(), None, gpui::Modifiers::default());
    click_debug_selector(cx, OPTION);
    assert_eq!(library(cx)["dark"], "deep");
    assert_eq!(library(cx)["light"], "pair");
    super::super::keys::press(cx, "escape");
    assert!(cx.debug_bounds(OPTION).is_none());
    assert!(cx.debug_bounds(CARD).is_some());
    click_debug_selector(cx, "theme-export-collection-studio.tide");
    let export = dir.path().join("deep.json");
    let path = export.clone();
    cx.simulate_new_path_selection(move |_| Some(path));
    cx.run_until_parked();
    let exported: serde_json::Value =
        serde_json::from_slice(&std::fs::read(export).unwrap()).unwrap();
    assert_eq!(exported["id"], "deep");
    let remove_center = rendered_bounds(cx, REMOVE).center();
    cx.simulate_mouse_down(
        remove_center,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    let remove_focus = super::super::keys::focused(cx).expect("Remove takes focus");
    cx.simulate_mouse_up(
        remove_center,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    assert!(cx.debug_bounds("desktop-settings-dialog").is_some());
    let dialog = workspace.read_with(cx, |ws, _| ws.modal_start_focus_handle.clone());
    for _ in 0..9 {
        cx.simulate_keystrokes("tab");
        assert!(
            cx.update(|window, cx| dialog.contains_focused(window, cx)),
            "Tab stays in variant removal"
        );
    }
    click_debug_selector(cx, "remove-variants-confirm");
    assert!(cx.debug_bounds("remove-variant-deep").is_some());
    click_debug_selector(cx, "remove-variant-pair");
    click_debug_selector(cx, "remove-variants-cancel");
    assert_eq!(library(cx)["themes"].as_array().unwrap().len(), 4);
    assert_eq!(
        super::super::keys::focused(cx).as_ref(),
        Some(&remove_focus),
        "Cancel returns focus to Remove themes"
    );
    click_debug_selector(cx, REMOVE);
    click_debug_selector(cx, "remove-variant-deep");
    click_debug_selector(cx, "remove-variant-midnight");
    click_debug_selector(cx, "remove-variants-confirm");
    assert!(cx.debug_bounds(CARD).is_none());
    assert!(cx.debug_bounds("settings-theme-custom-pair").is_some());
    assert_eq!(library(cx)["light"], "pair");
    assert_eq!(library(cx)["dark"], serde_json::Value::Null);
    saved(&workspace, cx).await;
    quit(&workspace, cx);
    let (workspace, cx) = launch(&mut relaunch, &state);
    workspace.read_with(cx, |ws, _| {
        let lib = serde_json::to_value(&ws.custom_themes).unwrap();
        assert_eq!(lib["light"], "pair");
        assert!(lib["dark"].is_null());
        let ids: Vec<_> = lib["themes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["pair", "custom-sand"]);
    });
}
