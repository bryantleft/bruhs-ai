use super::*;

#[test]
fn captured_media_survives_deleted_projects_and_signatures_are_asset_scoped() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(project.join("image.png"), b"image bytes").unwrap();
    let store = RichStore::at(directory.path().join("responses"));
    let project_str = project.to_str().unwrap();
    let asset = store.capture("image.png", Some(project_str)).unwrap();
    std::fs::remove_dir_all(&project).unwrap();
    let reopened = store.capture("image.png", Some(project_str)).unwrap();
    assert_eq!(asset.src, reopened.src);
    assert_eq!(std::fs::read(&asset.src).unwrap(), b"image bytes");
    let name = Path::new(&asset.src).file_name().unwrap().to_str().unwrap();
    let token = asset.signed_url.split_once("?token=").unwrap().1;
    assert!(store.authorized_asset(name, token).is_ok());
    assert!(store
        .authorized_asset(&format!("{}.mp4", &name[..64]), token)
        .is_err());
    assert!(store.authorized_asset("../secret", token).is_err());
}

#[test]
fn state_reopens_and_refuses_path_keys_or_oversized_values() {
    let directory = tempfile::tempdir().unwrap();
    let store = RichStore::at(directory.path());
    let key = widget_state_key("thread/message/0", "source");
    let mut state = WidgetState {
        custom: serde_json::json!({"counter":3}),
        ..Default::default()
    };
    store.save_state(&key, &state).unwrap();
    assert_eq!(
        RichStore::at(directory.path()).load_state(&key).unwrap(),
        state
    );
    assert!(store.save_state("../../other.json", &state).is_err());
    state.custom = serde_json::json!("x".repeat(MAX_WIDGET_STATE_BYTES));
    assert!(store.save_state(&key, &state).is_err());
}

#[test]
fn capture_rejects_foreign_paths_and_preserves_valid_vtt() {
    let directory = tempfile::tempdir().unwrap();
    let store = RichStore::at(directory.path().join("responses"));
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(directory.path().join("foreign.png"), b"foreign").unwrap();
    std::fs::write(
        project.join("captions.vtt"),
        b"WEBVTT\n\n00:00.000 --> 00:01.000\nHello",
    )
    .unwrap();
    assert!(store
        .capture("../foreign.png", Some(project.to_str().unwrap()))
        .is_err());
    let file = store
        .capture("captions.vtt", Some(project.to_str().unwrap()))
        .unwrap();
    assert_eq!(media_mime(Path::new(&file.src)), "text/vtt");
}

#[test]
fn saved_replies_keep_distinct_bytes_when_a_project_filename_is_reused() {
    use bruhs_models::logic::rich_response::{
        response_blocks, rewrite_response_media, ResponseBlock,
    };
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let store = RichStore::at(dir.path().join("responses"));
    let path = project.join("output.png");
    std::fs::write(&path, b"first version").unwrap();
    let first = store
        .capture("output.png", Some(project.to_str().unwrap()))
        .unwrap();
    let first_reply = rewrite_response_media(
        "![Result](output.png)",
        &std::collections::BTreeMap::from([("output.png".into(), first.src.clone())]),
    );
    std::fs::write(&path, b"second version").unwrap();
    let second = store
        .capture("output.png", Some(project.to_str().unwrap()))
        .unwrap();
    assert_ne!(first.src, second.src);
    std::fs::remove_dir_all(project).unwrap();
    let blocks = response_blocks(&first_reply, false);
    let ResponseBlock::Media(saved) = &blocks[0] else {
        panic!("saved media")
    };
    assert_eq!(std::fs::read(&saved.src).unwrap(), b"first version");
    assert_eq!(std::fs::read(second.src).unwrap(), b"second version");
}
