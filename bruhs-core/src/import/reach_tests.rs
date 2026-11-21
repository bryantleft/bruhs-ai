//! D15, rulings F21 and p6 F30: no bruh tool names an import route. The
//! helpers' tool lists are private functions, so this reads their source,
//! the same files Backup's reach test reads, plus every
//! `bruhs-mcp/src/tools/*.rs`. It is stricter than a literal scan: the
//! route prefixes may not appear anywhere in those files, comments
//! included, so no literal form (raw, escaped, split over lines) slips
//! past a scanner.

/// Every helper Bruhs starts, and the MCP server's tool list.
const SOURCES: [(&str, &str); 10] = [
    ("workbench_tools.rs", include_str!("../workbench_tools.rs")),
    ("thread_link.rs", include_str!("../thread_link.rs")),
    (
        "thread_link/whiteboards.rs",
        include_str!("../thread_link/whiteboards.rs"),
    ),
    ("secrets/mcp.rs", include_str!("../secrets/mcp.rs")),
    ("reactive_tools.rs", include_str!("../reactive_tools.rs")),
    ("database_tools.rs", include_str!("../database_tools.rs")),
    (
        "api_client_tools.rs",
        include_str!("../api_client_tools.rs"),
    ),
    ("website_reader.rs", include_str!("../website_reader.rs")),
    ("artifact_tools.rs", include_str!("../artifact_tools.rs")),
    (
        "bruhs-mcp/src/tools.rs",
        include_str!("../../../bruhs-mcp/src/tools.rs"),
    ),
];

/// The route prefixes no helper may name, in any case.
const ROUTES: [&str; 2] = ["api/import", "api/export"];

/// Every `bruhs-mcp/src/tools/*.rs` file, by name with its source, read
/// from disk so a new one is scanned too.
fn mcp_tool_files() -> Vec<(String, String)> {
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../bruhs-mcp/src/tools");
    let mut files: Vec<(String, String)> = std::fs::read_dir(&folder)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (
                format!("bruhs-mcp/src/tools/{name}"),
                std::fs::read_to_string(&path).unwrap(),
            )
        })
        .collect();
    files.sort();
    files
}

/// The route prefix `source` names, if any.
fn named_route(source: &str) -> Option<&'static str> {
    let lower = source.to_lowercase();
    ROUTES.into_iter().find(|route| lower.contains(route))
}

#[test]
fn the_guard_sees_a_route_however_it_is_written() {
    for source in [
        r#"let url = format!("{base}/api/import/uploads");"#,
        r##"const PATH: &str = r#"/API/Export/t3"#;"##,
        "// calls /api/import/thread",
    ] {
        assert!(named_route(source).is_some(), "{source}");
    }
    assert_eq!(named_route(r#"let path = "/api/backup";"#), None);
}

#[test]
fn no_bruh_tool_reaches_imports() {
    let tools = mcp_tool_files();
    assert!(
        tools.len() >= 4,
        "the MCP server's tool files: {}",
        tools.len()
    );
    let sources = SOURCES.iter().map(|(file, source)| (*file, *source)).chain(
        tools
            .iter()
            .map(|(file, source)| (file.as_str(), source.as_str())),
    );
    for (file, source) in sources {
        assert!(source.contains("fn "), "{file} reads as no source");
        assert_eq!(named_route(source), None, "{file} names an import route");
    }
}
