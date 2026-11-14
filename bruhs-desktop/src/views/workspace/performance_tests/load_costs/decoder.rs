//! Compare full saved-state readers while holding construction and rendering constant.
use super::super::super::{
    desktop_state_backup_path, persistence::decode_desktop_state, read_desktop_state_snapshot,
    state_file, DiskVersion, ThreadRestorer,
};
use super::*;
use std::{fs, io};

mod backup;
mod project_path;
mod reference;

const CASES: [&str; 18] = [
    "settings-control",
    "knowledge-small",
    "knowledge-large",
    "records-large",
    "history-control",
    "meeting-control",
    "mixed",
    "thread-rich-small",
    "thread-rich-history",
    "thread-assistant-history",
    "thread-single-history",
    "thread-many-small",
    "ascii-compact",
    "ascii-pretty",
    "unicode-compact",
    "unicode-pretty",
    "escaped-compact",
    "escaped-pretty",
];

fn fixture_state(label: &str) -> PersistedWorkspaceState {
    match label {
        "settings-control" => saved_state(label, 0, 0, 0),
        "knowledge-small" => saved_state(label, 256, 0, 0),
        "knowledge-large" => saved_state(label, 4096, 0, 0),
        "records-large" => saved_state(label, 0, 4096, 0),
        "history-control" => saved_state(label, 0, 0, 8192),
        "meeting-control" => saved_state(label, 0, 0, 0),
        "mixed" => saved_state(label, 1024, 1024, 2048),
        label if label.starts_with("thread-") => threads::thread_state(label),
        label => {
            let mut state = saved_state(label, 256, 0, 1024);
            let text = match label.split_once('-').unwrap().0 {
                "ascii" => "Saved ASCII workspace text and symbols. ".repeat(48),
                "unicode" => "Saved café, 👩🏽‍💻, 日本語 and العربية. ".repeat(48),
                "escaped" => "Saved café, 👩🏽‍💻, quotes \" tabs\t and lines\n".repeat(48),
                _ => panic!("unknown decoder fixture {label}"),
            };
            for document in &mut state.knowledge_documents {
                document.body_markdown.clone_from(&text);
                for version in &mut document.versions {
                    version.body_markdown.clone_from(&text);
                }
            }
            for thread in &mut state.threads {
                for message in &mut thread.messages {
                    message.content.clone_from(&text);
                }
            }
            state
        }
    }
}

fn fixture_bytes(label: &str) -> Vec<u8> {
    let state = fixture_state(label);
    if label.ends_with("-pretty") {
        serde_json::to_vec_pretty(&state).unwrap()
    } else {
        serde_json::to_vec(&state).unwrap()
    }
}

fn state_value(state: PersistedWorkspaceState) -> Value {
    let mut value = serde_json::to_value(state).unwrap();
    normalize_fresh_rule_ids(&mut value);
    value
}

fn assert_decode_parity(bytes: &[u8]) {
    let expected = serde_json::from_slice::<PersistedWorkspaceState>(bytes);
    let actual = decode_desktop_state(bytes);
    match (expected, actual) {
        (Ok(expected), Ok(actual)) => assert_eq!(state_value(actual), state_value(expected)),
        (Err(expected), Err(actual)) => {
            assert_eq!(actual.to_string(), expected.to_string());
            assert_eq!(actual.classify(), expected.classify());
            assert_eq!(
                (actual.line(), actual.column()),
                (expected.line(), expected.column())
            );
        }
        (expected, actual) => panic!("decoder acceptance changed: {expected:?} / {actual:?}"),
    }
}

#[test]
fn saved_state_decoder_preserves_schema_unicode_escapes_and_errors() {
    for input in [
        "{}",
        " \n{}\r\n",
        "null",
        "[]",
        "",
        "{",
        "{}{}",
        "{\"unknown\":[1,true,null]}",
        r#"{"current_model":"café 👩🏽‍💻 日本語 العربية","unknown":{"nested":[1.25]}}"#,
        r#"{"current_model":"\uD83D\uDE00\n\t\"\\\u0000"}"#,
        r#"{"current_model":"\uD800"}"#,
        r#"{"current_model":"\uDE00"}"#,
        r#"{"current_model":"\u00xz"}"#,
        r#"{"current_model":true}"#,
        r#"{"permission_mode":"future_value","sidebar_tab":"future_value"}"#,
        r#"{"threads":[{"id":"a","title":"b","project_name":"","model_id":""}]}"#,
        r#"{"thread_hops":1e999}"#,
        r#"{"current_model":"a","current_model":"b"}"#,
        "{\"current_model\":\"raw\ncontrol\"}",
    ] {
        assert_decode_parity(input.as_bytes());
    }
    for length in 0..130 {
        let bytes =
            serde_json::to_vec(&json!({"current_model":format!("{}é😀", "a".repeat(length))}))
                .unwrap();
        assert_decode_parity(&bytes);
        for index in [bytes.len() / 2, bytes.len() - 1] {
            let mut invalid = bytes.clone();
            invalid[index] = 0xff;
            assert_decode_parity(&invalid);
        }
    }
    for depth in [127, 128, 129, 256] {
        let input = format!(
            "{{\"unknown\":{}0{}}}",
            "[".repeat(depth),
            "]".repeat(depth)
        );
        assert_decode_parity(input.as_bytes());
    }
}

#[test]
fn saved_state_decoder_retains_legacy_invalid_utf8_unknown_field_policy() {
    for bytes in [
        &b"{\"unknown\":\"\xff\"}"[..],
        &b"{\"unknown\":{\"ignored\":\"\xed\xa0\x80\"}}"[..],
    ] {
        assert!(serde_json::from_slice::<PersistedWorkspaceState>(bytes).is_ok());
        assert_decode_parity(bytes);
    }
    for bytes in [
        &b"{\"current_model\":\"\xff\"}"[..],
        &b"{\"current_model\":\"\xed\xa0\x80\"}"[..],
        &b"{\"current_model\":\"\xf4\x90\x80\x80\"}"[..],
        &b"{\"current_model\":\"\xc0\x80\"}"[..],
        &b"{\"current_model\":\"\xe2\x82\"}"[..],
    ] {
        assert!(serde_json::from_slice::<PersistedWorkspaceState>(bytes).is_err());
        assert_decode_parity(bytes);
    }
}

#[test]
fn saved_state_decoder_preserves_full_rich_state_and_reproducible_inputs() {
    for label in [
        "thread-rich-small",
        "ascii-pretty",
        "unicode-compact",
        "escaped-pretty",
    ] {
        let bytes = fixture_bytes(label);
        assert_eq!(bytes, fixture_bytes(label));
        let expected = serde_json::from_slice::<PersistedWorkspaceState>(&bytes).unwrap();
        let actual = decode_desktop_state(&bytes).unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }
}

#[test]
fn saved_file_decoder_preserves_versions_errors_and_backup_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("saved café.json");
    let backup = desktop_state_backup_path(&path);
    let valid = fixture_bytes("thread-rich-small");
    fs::write(&backup, &valid).unwrap();
    for bytes in [
        Some(valid.as_slice()),
        Some(&b"{ bad primary"[..]),
        Some(&b"{\"current_model\":\"\xff\"}"[..]),
        Some(&b""[..]),
        None,
    ] {
        if let Some(bytes) = bytes {
            fs::write(&path, bytes).unwrap();
        } else {
            fs::remove_file(&path).unwrap();
        }
        let expected = reference::load_original_state_with_backup(&path);
        let actual = load_desktop_state_with_backup(&path);
        assert_eq!(actual.0.map(state_value), expected.0.map(state_value));
        assert_eq!(actual.1, expected.1);
        assert_eq!(actual.2, expected.2);
        if let Some(bytes) = bytes {
            if bytes == valid {
                assert_eq!(actual.2, DiskVersion::of_bytes(&valid));
            }
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        assert_eq!(fs::read(&backup).unwrap(), valid);
    }
    fs::create_dir(&path).unwrap();
    assert_eq!(
        read_desktop_state_snapshot(&path).unwrap_err(),
        reference::read_original_state_snapshot(&path).unwrap_err()
    );
    fs::remove_dir(&path).unwrap();
    fs::write(&path, b"{").unwrap();
    fs::write(&backup, b"{").unwrap();
    let expected = reference::load_original_state_with_backup(&path);
    let actual = load_desktop_state_with_backup(&path);
    assert!(actual.0.is_none());
    assert_eq!((actual.1, actual.2), (expected.1, expected.2));
}

#[gpui::test]
#[ignore = "manual optimized full-file decoder comparison through shared construction and settling"]
fn complete_workspace_decode_load_cost(cx: &mut TestAppContext) {
    initialize(cx);
    let (variant, load): (&str, LoadState) =
        match std::env::var("BRUHS_PERF_STATE_DECODER").as_deref() {
            Ok("original") => ("original", reference::load_original_state_with_backup),
            Ok("simd") | Err(_) => ("simd", load_desktop_state_with_backup),
            other => panic!("invalid state decoder {other:?}"),
        };
    let selected = std::env::var("BRUHS_PERF_DECODER_CASE").ok();
    measure_load_cases(
        cx,
        variant,
        load,
        reference::load_original_state_with_backup,
        selected.as_deref(),
        "DECODE_LOAD_COST",
    );
}

fn measure_load_cases(
    cx: &mut TestAppContext,
    variant: &str,
    load: LoadState,
    expected_load: LoadState,
    selected: Option<&str>,
    marker: &str,
) {
    measure_load_cases_with_threads(
        cx,
        variant,
        load,
        expected_load,
        (threads::current_threads, threads::current_threads),
        selected,
        marker,
    );
}

fn measure_load_cases_with_threads(
    cx: &mut TestAppContext,
    variant: &str,
    load: LoadState,
    expected_load: LoadState,
    (restore_threads, expected_threads): (ThreadRestorer, ThreadRestorer),
    selected: Option<&str>,
    marker: &str,
) {
    let root = tempfile::tempdir().unwrap();
    let mut measured = 0;
    for label in CASES {
        if selected.is_some_and(|selected| selected != label) {
            continue;
        }
        let bytes = fixture_bytes(label);
        for recovery in [false, true] {
            let path = root.path().join(format!("{label}-{recovery}.json"));
            if recovery {
                fs::write(&path, b"{ invalid primary").unwrap();
                fs::write(desktop_state_backup_path(&path), &bytes).unwrap();
            } else {
                fs::write(&path, &bytes).unwrap();
            }
            let expected = observe_with_loader(cx, &path, current, expected_threads, expected_load);
            let (mut total, mut read, mut construct, mut settle) =
                (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            for iteration in 0..35 {
                let actual = observe_with_loader(cx, &path, current, restore_threads, load);
                assert_eq!(
                    actual.snapshot, expected.snapshot,
                    "{label}/{recovery}/{iteration}"
                );
                assert_eq!(actual.recovered, recovery);
                if iteration >= 5 {
                    total.push(actual.total_ms);
                    read.push(actual.read_decode_hash_ms);
                    construct.push(actual.construct_ms);
                    settle.push(actual.settle_ms);
                }
            }
            assert_eq!(
                fs::read(if recovery {
                    desktop_state_backup_path(&path)
                } else {
                    path.clone()
                })
                .unwrap(),
                bytes
            );
            if recovery {
                assert_eq!(fs::read(&path).unwrap(), b"{ invalid primary");
            }
            println!(
                "{marker} {}",
                json!({"label":label,"recovery":recovery,"variant":variant,"input_bytes":bytes.len(),"input_sha256":format!("{:x}",Sha256::digest(&bytes)),"warmups":5,"total_samples_ms":total,"load_samples_ms":read,"construct_samples_ms":construct,"settle_samples_ms":settle})
            );
            measured += 1;
        }
    }
    assert!(measured > 0, "selected decoder fixture must exist");
}

#[test]
#[ignore = "manual optimized decoder-only mechanism comparison, not complete-action evidence"]
fn workspace_decode_component_cost() {
    for label in CASES {
        let bytes = fixture_bytes(label);
        let expected = serde_json::from_slice::<PersistedWorkspaceState>(&bytes).unwrap();
        let expected = serde_json::to_value(expected).unwrap();
        for mode in ["slice", "std", "simd", "simd", "std", "slice"] {
            let mut samples = Vec::new();
            for iteration in 0..35 {
                let start = Instant::now();
                let state = match mode {
                    "slice" => serde_json::from_slice::<PersistedWorkspaceState>(black_box(&bytes))
                        .unwrap(),
                    "std" => serde_json::from_str(std::str::from_utf8(black_box(&bytes)).unwrap())
                        .unwrap(),
                    _ => decode_desktop_state(black_box(&bytes)).unwrap(),
                };
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                assert_eq!(serde_json::to_value(state).unwrap(), expected);
                if iteration >= 5 {
                    samples.push(elapsed);
                }
            }
            println!(
                "DECODE_COMPONENT_COST {}",
                json!({"label":label,"variant":mode,"input_bytes":bytes.len(),"input_sha256":format!("{:x}",Sha256::digest(&bytes)),"warmups":5,"decode_samples_ms":samples})
            );
        }
    }
}
