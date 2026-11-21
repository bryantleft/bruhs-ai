//! Helpers for import tests in this crate and in the server's tests
//! (`test-support` feature): export archives built in code, and the
//! synthetic fixtures under `src/import/fixtures/`.

use std::io::{Cursor, Write as _};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// A ZIP holding `entries` in order, deflated or stored.
///
/// # Panics
///
/// When the writer refuses an entry (a repeated name).
pub fn zip_of(entries: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
    let method = if deflate {
        CompressionMethod::Deflated
    } else {
        CompressionMethod::Stored
    };
    let options = SimpleFileOptions::default()
        .compression_method(method)
        .large_file(false);
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        writer
            .start_file(*name, options)
            .expect("a test entry starts");
        writer.write_all(bytes).expect("a test entry is written");
    }
    writer.finish().expect("a test ZIP finishes").into_inner()
}

/// Every fixture under `src/import/fixtures/`, by file name. All are
/// hand-written and synthetic (`fixtures_are_synthetic`).
const FIXTURES: &[(&str, &[u8])] = &[
    (
        "chatgpt-conversations.json",
        include_bytes!("fixtures/chatgpt-conversations.json"),
    ),
    (
        "chatgpt-export_manifest.json",
        include_bytes!("fixtures/chatgpt-export_manifest.json"),
    ),
    (
        "claude-conversations.json",
        include_bytes!("fixtures/claude-conversations.json"),
    ),
    (
        "claude-memories.json",
        include_bytes!("fixtures/claude-memories.json"),
    ),
    (
        "claude-memories-account.json",
        include_bytes!("fixtures/claude-memories-account.json"),
    ),
    (
        "claude-manifest.json",
        include_bytes!("fixtures/claude-manifest.json"),
    ),
    (
        "grok-prod-grok-backend.json",
        include_bytes!("fixtures/grok-prod-grok-backend.json"),
    ),
];

/// The fixture file `name`, from `src/import/fixtures/`.
///
/// # Panics
///
/// When no fixture has that name.
pub fn fixture(name: &str) -> &'static [u8] {
    match FIXTURES.iter().find(|(fixture, _)| *fixture == name) {
        Some((_, bytes)) => bytes,
        None => panic!("no import fixture named {name}"),
    }
}
