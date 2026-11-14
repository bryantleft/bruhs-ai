use super::*;

fn fixture() -> (tempfile::TempDir, PathBuf, ArtifactStore) {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let store = ArtifactStore::at(directory.path().join("saved"));
    (directory, project, store)
}

#[test]
fn publication_bundles_local_css_scripts_images_fonts_and_media_and_survives_source_deletion() {
    let (_directory, project, store) = fixture();
    std::fs::create_dir(project.join("assets")).unwrap();
    std::fs::write(project.join("assets/photo.png"), b"picture").unwrap();
    std::fs::write(project.join("assets/tone.wav"), b"audio").unwrap();
    std::fs::write(project.join("assets/captions.vtt"), b"WEBVTT\n").unwrap();
    std::fs::write(project.join("assets/font.woff2"), b"font").unwrap();
    std::fs::write(
        project.join("assets/base.css"),
        "@font-face{font-family:Demo;src:url(font.woff2)}",
    )
    .unwrap();
    std::fs::write(
        project.join("assets/style.css"),
        "@import 'base.css'; .photo{background:url(\"photo.png\")} @media screen{body{color:red}}",
    )
    .unwrap();
    std::fs::write(
        project.join("assets/app.js"),
        "document.querySelector('button').onclick=()=>{document.body.dataset.clicked='yes'};",
    )
    .unwrap();
    std::fs::write(project.join("page.html"), "<!doctype html><title>Local demo</title><link rel=stylesheet href=assets/style.css media=screen><button>Try</button><script src=assets/app.js></script><img src=assets/photo.png><audio src=assets/tone.wav controls><track src=assets/captions.vtt kind=captions></audio>").unwrap();
    let artifact = store.publish("page.html", None, project.to_str()).unwrap();
    assert_eq!(artifact.title, "Local demo");
    assert!(artifact.html.contains("data:image/png;base64,cGljdHVyZQ=="));
    assert!(artifact.html.contains("data:audio/wav;base64,YXVkaW8="));
    assert!(artifact.html.contains("data:text/vtt;base64,"));
    assert!(artifact.html.contains("data:font/woff2;base64,Zm9udA=="));
    assert!(artifact.html.contains("<style media=\"screen\">"));
    assert!(artifact.html.contains("body{color:red}"));
    assert!(artifact.html.contains("dataset.clicked='yes'"));
    assert!(!artifact.html.contains("assets/"));
    std::fs::remove_dir_all(&project).unwrap();
    assert_eq!(store.load(&artifact.id).unwrap(), artifact);
    assert_eq!(
        store.load_diagnostics(&artifact.id).unwrap(),
        ArtifactDiagnostics::default()
    );
}

#[test]
fn dependencies_cannot_escape_scope_or_silently_remain_missing_or_remote() {
    let (directory, project, store) = fixture();
    std::fs::write(directory.path().join("outside.png"), b"private").unwrap();
    for (html, expected) in [
        ("<img src=../outside.png>", "outside the current project"),
        ("<img src=missing.png>", "unavailable"),
        (
            "<script src=https://example.com/script.js></script>",
            "External dependency",
        ),
        (
            "<style>body{background:url(https://example.com/tracker)}</style>",
            "External dependency",
        ),
        (
            "<script type=module>import x from './module.js'</script>",
            "prebundled",
        ),
        ("<script>fetch('./data.json')</script>", "inline data"),
    ] {
        std::fs::write(project.join("page.html"), html).unwrap();
        let error = store
            .publish("page.html", None, project.to_str())
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{html}: {error}");
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            directory.path().join("outside.png"),
            project.join("escape.png"),
        )
        .unwrap();
        std::fs::write(project.join("page.html"), "<img src=escape.png>").unwrap();
        assert!(store
            .publish("page.html", None, project.to_str())
            .unwrap_err()
            .to_string()
            .contains("outside the current project"));
    }
}

#[test]
fn tokenizer_does_not_mistake_script_comments_strings_or_regex_for_imports() {
    let (_directory, project, store) = fixture();
    std::fs::write(
        project.join("page.html"),
        r#"<script>// import './not-a-dependency.js'
const explanation = "fetch and import are words";
const pattern = /import\s+thing/;
document.body.dataset.ready='yes';</script>"#,
    )
    .unwrap();
    assert!(store.publish("page.html", None, project.to_str()).is_ok());
}

#[test]
fn metadata_and_diagnostics_are_durable_and_cannot_be_mistaken_for_a_preview() {
    let (_directory, project, store) = fixture();
    std::fs::write(project.join("page.html"), "<button>Try</button>").unwrap();
    let artifact = store
        .publish("page.html", Some("Demo"), project.to_str())
        .unwrap();
    assert!(store
        .load_diagnostics(&artifact.id)
        .unwrap()
        .previewed_at_ms
        .is_none());
    let errors = vec!["ReferenceError: missingChart is not defined".into()];
    let report = store
        .record_diagnostics(&artifact.id, PreviewState::Error, errors.clone())
        .unwrap();
    assert_eq!(report.errors, errors);
    assert!(report.previewed_at_ms.is_some());
    assert_eq!(store.load_diagnostics(&artifact.id).unwrap(), report);
    assert!(store
        .record_diagnostics(&artifact.id, PreviewState::Ready, errors)
        .is_err());
    assert!(store.load("../page.html").is_err());
    assert!(store.load_diagnostics("../../../secrets").is_err());
    assert!(store.root.read_dir().unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .ends_with(".tmp")));
}

#[test]
fn recursive_css_imports_are_flattened_and_cycles_report_the_dependency() {
    let (_directory, project, store) = fixture();
    std::fs::write(
        project.join("page.html"),
        "<link rel=stylesheet href=a.css>",
    )
    .unwrap();
    std::fs::write(
        project.join("a.css"),
        "@import 'b.css' screen; body{margin:0}",
    )
    .unwrap();
    std::fs::write(project.join("b.css"), "button{color:orange}").unwrap();
    let artifact = store.publish("page.html", None, project.to_str()).unwrap();
    assert!(artifact
        .html
        .contains("@media screen{button{color:orange}}"));
    std::fs::write(project.join("b.css"), "@import 'a.css';").unwrap();
    assert!(store
        .publish("page.html", None, project.to_str())
        .unwrap_err()
        .to_string()
        .contains("import cycle"));
}

#[test]
fn bundled_deferred_script_still_runs_after_its_target_dom_exists() {
    let (_directory, project, store) = fixture();
    std::fs::write(project.join("page.html"), "<!doctype html><head><script defer src=first.js></script><script defer src=second.js></script></head><body><button id=target>Try</button></body>").unwrap();
    std::fs::write(
        project.join("first.js"),
        "window.first=document.querySelector('#target');",
    )
    .unwrap();
    std::fs::write(
        project.join("second.js"),
        "window.first.textContent='Ready';",
    )
    .unwrap();
    let artifact = store.publish("page.html", None, project.to_str()).unwrap();
    assert!(
        artifact.html.find("id=\"target\"").unwrap() < artifact.html.find("window.first=").unwrap()
    );
    assert!(
        artifact.html.find("window.first=").unwrap()
            < artifact.html.find("window.first.textContent=").unwrap()
    );
}
