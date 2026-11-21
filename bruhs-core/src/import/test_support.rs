//! Helpers for import tests in this crate and in the server's tests
//! (`test-support` feature): export archives built in code.

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
