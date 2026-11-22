use super::*;
use std::io::{Seek, Write};

#[test]
fn search_ranks_identity_authors_and_descriptions_and_offers_only_compatible_installs() {
    let rows = serde_json::json!({"agents":[
        {"id":"assistant","name":"Writer Helper","version":"2","authors":["Test Team"],"description":"writes documents","distribution":{"binary":{"darwin-aarch64":{"cmd":"./bin/tool","archive":"https://example.test/build.zip"}},"npx":{"package":"example@1.0.0"}}},
        {"id":"python","name":"Python","version":"3","distribution":{"uvx":{"package":"python-tool@3.0.0"}}},
        {"id":"unstable","name":"Unpinned","version":"2","distribution":{"npx":{"package":"pkg@latest"}}}
    ]});
    let bytes = serde_json::to_vec(&rows).unwrap();
    let agents = parse(&bytes, "darwin-aarch64", true, true).unwrap();
    assert_eq!(agents.len(), 2);
    let a = agents.iter().find(|a| a.id == "assistant").unwrap();
    assert_eq!(a.method(), "Binary");
    assert!(a.search_rank("writer help") < a.search_rank("test team"));
    assert!(a.search_rank("writer documents").is_some());
    assert_eq!(a.search_rank("does not exist"), None);
    assert_eq!(
        parse(&bytes, "other", true, false).unwrap()[0].method(),
        "npm"
    );
    assert!(parse(&bytes, "other", false, false).unwrap().is_empty());
}

fn tar_file(path: &str, target: Option<&str>) -> std::fs::File {
    let file = tempfile::tempfile().unwrap();
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
    let mut tar = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    header.set_mode(0o755);
    header.set_size(4);
    header.set_cksum();
    tar.append_data(&mut header, path, b"test".as_slice())
        .unwrap();
    if let Some(target) = target {
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(tar::EntryType::Symlink);
        h.set_size(0);
        h.set_mode(0o777);
        tar.append_link(&mut h, "bin/entry", target).unwrap();
    }
    let mut file = tar.into_inner().unwrap().finish().unwrap();
    file.rewind().unwrap();
    file
}

#[cfg(unix)]
#[test]
fn binary_install_keeps_internal_links_and_rejects_external_targets() {
    let dir = tempfile::tempdir().unwrap();
    archive::unpack(
        &tar_file("lib/agent", Some("../lib/agent")),
        "agent.tgz",
        dir.path(),
        Path::new("bin/entry"),
    )
    .unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("bin/entry")).unwrap(),
        b"test"
    );
    let other = tempfile::tempdir().unwrap();
    assert!(archive::unpack(
        &tar_file("lib/agent", Some("../../outside")),
        "agent.tgz",
        other.path(),
        Path::new("bin/entry")
    )
    .is_err());
    assert!(!other.path().join("bin/entry").exists());
}

#[test]
fn zip_raw_and_bzip2_install_at_the_declared_command_path() {
    let dir = tempfile::tempdir().unwrap();
    let file = tempfile::tempfile().unwrap();
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file(
        "payload/agent",
        zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated),
    )
    .unwrap();
    zip.write_all(b"zip").unwrap();
    let mut file = zip.finish().unwrap();
    file.rewind().unwrap();
    archive::unpack(&file, "agent.zip", dir.path(), Path::new("payload/agent")).unwrap();
    let mut raw = tempfile::tempfile().unwrap();
    raw.write_all(b"raw").unwrap();
    raw.rewind().unwrap();
    archive::unpack(&raw, "download", dir.path(), Path::new("nested/raw")).unwrap();
    let tar = flate2::read::GzDecoder::new(tar_file("lib/binary", None));
    let mut compressed =
        bzip2::write::BzEncoder::new(tempfile::tempfile().unwrap(), bzip2::Compression::fast());
    std::io::copy(&mut std::io::BufReader::new(tar), &mut compressed).unwrap();
    let mut file = compressed.finish().unwrap();
    file.rewind().unwrap();
    archive::unpack(&file, "tool.tar.bz2", dir.path(), Path::new("lib/binary")).unwrap();
    for (path, expected) in [
        ("payload/agent", "zip"),
        ("nested/raw", "raw"),
        ("lib/binary", "test"),
    ] {
        assert_eq!(
            std::fs::read_to_string(dir.path().join(path)).unwrap(),
            expected
        );
    }
}
