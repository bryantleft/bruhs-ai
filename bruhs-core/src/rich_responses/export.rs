//! Portable reply bundles keep media files on disk while writing the ZIP.
use super::{RichError, RichStore};
use bruhs_models::logic::rich_response::{
    response_blocks, standalone_widget_document, widget_state_key, MediaBlock, MediaKind,
    ResponseBlock, WidgetState,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::PathBuf,
};

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExportRequest {
    pub markdown: String,
    pub project: Option<String>,
    pub scope: String,
    pub state_keys: Vec<String>,
    pub widget_states: BTreeMap<String, WidgetState>,
    pub include_state: bool,
    pub include_remote: bool,
}

#[derive(Default, Serialize)]
struct Manifest {
    external_references: Vec<String>,
    widget_states_included: bool,
}

pub async fn build(
    store: RichStore,
    request: ExportRequest,
) -> Result<tempfile::NamedTempFile, RichError> {
    let blocks = response_blocks(&request.markdown, false);
    let mut remote_urls = BTreeSet::new();
    // This visits images in tables/lists too without splitting containers.
    for source in bruhs_models::logic::rich_response::all_media_sources(&request.markdown) {
        if bruhs_models::logic::rich_response::media_is_remote(&source) {
            remote_urls.insert(source);
        }
    }
    let mut remote = BTreeMap::new();
    if request.include_remote {
        if remote_urls.len() > bruhs_models::logic::attachments::MAX_FILES_PER_MESSAGE {
            return Err(RichError::Invalid("A reply bundle supports at most 100 external media files, matching the attachment budget.".into()));
        }
        // Download each source to a bounded temporary file; archive writing
        // consumes immutable files rather than retaining their bytes in RAM.
        let operations = remote_urls.into_iter().map(|url| {
            let store = store.clone();
            async move {
                let asset = super::remote::download(&store, &url).await?;
                Ok::<_, RichError>((url, asset.src))
            }
        });
        use futures_util::StreamExt;
        let mut pending = std::pin::pin!(futures_util::stream::iter(operations)
            .buffer_unordered(bruhs_models::logic::attachments::MAX_FILES_PER_MESSAGE));
        while let Some(result) = pending.next().await {
            let (url, path) = result?;
            remote.insert(url, path);
        }
    }
    tokio::task::spawn_blocking(move || write_bundle(&store, request, blocks, remote))
        .await
        .map_err(|e| RichError::Invalid(e.to_string()))?
}

fn write_bundle(
    store: &RichStore,
    request: ExportRequest,
    blocks: Vec<ResponseBlock>,
    remote: BTreeMap<String, String>,
) -> Result<tempfile::NamedTempFile, RichError> {
    let file = tempfile::NamedTempFile::new()?;
    let mut archive = zip::ZipWriter::new(file.reopen()?);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .large_file(true);
    let mut files = BTreeMap::<String, PathBuf>::new();
    let mut markdown = String::new();
    let mut html = String::new();
    let mut manifest = Manifest {
        widget_states_included: request.include_state,
        ..Default::default()
    };
    let mut paths = BTreeMap::new();
    for source in bruhs_models::logic::rich_response::all_media_sources(&request.markdown) {
        let target = bundle_source(
            store,
            &source,
            request.project.as_deref(),
            &remote,
            &mut files,
            &mut manifest,
        )?;
        paths.insert(source, target);
    }
    let mut widget_index = 0_usize;
    let theme = BTreeMap::new();
    for (index, block) in blocks.into_iter().enumerate() {
        match block {
            ResponseBlock::Markdown(text) => {
                let text = bruhs_models::logic::rich_response::rewrite_media_paths(&text, &paths);
                markdown.push_str(&text);
                html.push_str(&format!("<pre>{}</pre>", attr(&text)));
            }
            ResponseBlock::Error { source, .. } => {
                markdown.push_str(&source);
                html.push_str(&format!("<pre>{}</pre>", attr(&source)));
            }
            ResponseBlock::Media(mut media) => {
                media.src = mapped(&paths, &media.src)?;
                if let Some(poster) = &mut media.poster {
                    *poster = mapped(&paths, poster)?;
                }
                for track in &mut media.tracks {
                    track.src = mapped(&paths, &track.src)?;
                }
                markdown.push_str(&format!(
                    "\n\n```bruhs-media\n{}\n```\n\n",
                    serde_json::to_string(&media)?
                ));
                html.push_str(&media_html(&media));
            }
            ResponseBlock::Html(block) => {
                let state = export_state(store, &request, index, widget_index, &block.html)?;
                widget_index += 1;
                write_widget(
                    &mut archive,
                    options,
                    &mut markdown,
                    &mut html,
                    &block.html,
                    &block.title,
                    &state,
                    &theme,
                    index,
                )?;
            }
            ResponseBlock::Artifact(block) => {
                let artifact = crate::artifacts::load(&block.id)
                    .map_err(|error| RichError::Invalid(error.to_string()))?;
                let state = export_state(store, &request, index, widget_index, &artifact.html)?;
                widget_index += 1;
                write_widget(
                    &mut archive,
                    options,
                    &mut markdown,
                    &mut html,
                    &artifact.html,
                    &artifact.title,
                    &state,
                    &theme,
                    index,
                )?;
            }
        }
    }
    for (name, path) in files {
        archive.start_file(name, options).map_err(zip_error)?;
        std::io::copy(&mut std::fs::File::open(path)?, &mut archive)?;
    }
    archive.start_file("reply.md", options).map_err(zip_error)?;
    archive.write_all(markdown.as_bytes())?;
    archive
        .start_file("manifest.json", options)
        .map_err(zip_error)?;
    serde_json::to_writer(&mut archive, &manifest)?;
    archive
        .start_file("index.html", options)
        .map_err(zip_error)?;
    archive.write_all(format!("<!doctype html><meta charset=\"utf-8\"><meta name=\"referrer\" content=\"no-referrer\"><title>Saved reply</title><style>body{{max-width:900px;margin:24px auto;padding:12px;font:14px system-ui;overflow-wrap:anywhere}}pre{{white-space:pre-wrap}}img,video,iframe{{max-width:100%}}iframe{{width:100%;height:480px;border:0}}</style>{html}").as_bytes())?;
    archive.finish().map_err(zip_error)?.sync_all()?;
    Ok(file)
}

fn mapped(paths: &BTreeMap<String, String>, source: &str) -> Result<String, RichError> {
    paths
        .get(source)
        .cloned()
        .ok_or_else(|| RichError::Invalid("Media discovery missed a referenced asset.".into()))
}

fn bundle_source(
    store: &RichStore,
    source: &str,
    project: Option<&str>,
    remote: &BTreeMap<String, String>,
    files: &mut BTreeMap<String, PathBuf>,
    manifest: &mut Manifest,
) -> Result<String, RichError> {
    let captured = if bruhs_models::logic::rich_response::media_is_remote(source) {
        match remote.get(source) {
            Some(path) => path.clone(),
            None => {
                manifest.external_references.push(source.into());
                return Ok(source.into());
            }
        }
    } else {
        store.capture(source, project)?.src
    };
    let path = PathBuf::from(captured);
    let name = format!(
        "media/{}",
        path.file_name()
            .ok_or_else(|| RichError::Invalid("Media has no filename.".into()))?
            .to_string_lossy()
    );
    files.insert(name.clone(), path);
    Ok(name)
}
fn export_state(
    store: &RichStore,
    request: &ExportRequest,
    index: usize,
    widget: usize,
    html: &str,
) -> Result<WidgetState, RichError> {
    if !request.include_state {
        return Ok(WidgetState::default());
    }
    let key = request
        .state_keys
        .get(widget)
        .cloned()
        .unwrap_or_else(|| widget_state_key(&format!("{}/{index}", request.scope), html));
    super::key_valid(&key)?;
    if let Some(state) = request.widget_states.get(&key) {
        state
            .validate()
            .map_err(|error| RichError::Invalid(error.into()))?;
        return Ok(state.clone());
    }
    store.load_state(&key)
}
#[expect(
    clippy::too_many_arguments,
    reason = "Archive output owns source, preview and optional state together."
)]
fn write_widget(
    archive: &mut zip::ZipWriter<std::fs::File>,
    options: zip::write::SimpleFileOptions,
    markdown: &mut String,
    preview: &mut String,
    source: &str,
    title: &str,
    state: &WidgetState,
    theme: &BTreeMap<String, String>,
    index: usize,
) -> Result<(), RichError> {
    let name = format!("widgets/{index}.html");
    archive.start_file(&name, options).map_err(zip_error)?;
    archive.write_all(standalone_widget_document(source, state, theme).as_bytes())?;
    let longest = source.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    markdown.push_str(&format!(
        "\n\n{fence}bruhs-html {}\n{source}\n{fence}\n\n",
        title.replace(['\r', '\n'], " ")
    ));
    preview.push_str(&format!("<section><h2>{}</h2><iframe sandbox=\"allow-scripts\" src=\"{}\" title=\"{}\"></iframe></section>",attr(title),name,attr(title)));
    Ok(())
}
fn media_html(media: &MediaBlock) -> String {
    let src = attr(&media.src);
    let title = attr(&media.title);
    let mut element = if bruhs_models::logic::rich_response::media_is_remote(&media.src) {
        format!("<a href=\"{src}\" rel=\"noreferrer noopener\">Open external media</a>")
    } else {
        match media.kind {
            MediaKind::Image => format!("<img src=\"{src}\" alt=\"{title}\">"),
            MediaKind::Audio | MediaKind::Video => {
                let tag = if media.kind == MediaKind::Audio {
                    "audio"
                } else {
                    "video"
                };
                let poster = media
                    .poster
                    .as_ref()
                    .filter(|source| !bruhs_models::logic::rich_response::media_is_remote(source))
                    .map_or(String::new(), |poster| {
                        format!(" poster=\"{}\"", attr(poster))
                    });
                let tracks = media
                    .tracks
                    .iter()
                    .filter(|track| {
                        !bruhs_models::logic::rich_response::media_is_remote(&track.src)
                    })
                    .map(|track| {
                        format!(
                            "<track src=\"{}\" label=\"{}\" srclang=\"{}\" kind=\"{}\">",
                            attr(&track.src),
                            attr(&track.label),
                            attr(&track.language),
                            attr(&track.kind)
                        )
                    })
                    .collect::<String>();
                format!("<{tag} controls preload=\"none\" src=\"{src}\"{poster}>{tracks}</{tag}>")
            }
            MediaKind::Pdf => format!("<a href=\"{src}\">{title}</a>"),
        }
    };
    if let Some(transcript) = &media.transcript {
        element.push_str(&format!(
            "<details><summary>Transcript</summary><pre>{}</pre></details>",
            attr(transcript)
        ))
    }
    format!("<figure><figcaption>{title}</figcaption>{element}</figure>")
}
fn attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn zip_error(error: zip::result::ZipError) -> RichError {
    RichError::Invalid(format!("Could not build the reply bundle: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    #[tokio::test]
    async fn zip_bundles_local_files_and_works_after_project_deletion() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        std::fs::write(project.join("p.png"), b"image").unwrap();
        let store = RichStore::at(dir.path().join("responses"));
        store
            .capture("p.png", Some(project.to_str().unwrap()))
            .unwrap();
        std::fs::remove_dir_all(&project).unwrap();
        let bundle = build(
            store,
            ExportRequest {
                markdown: "![Photo](p.png)".into(),
                project: Some(project.to_string_lossy().into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let mut archive = zip::ZipArchive::new(bundle.reopen().unwrap()).unwrap();
        assert!(archive.by_name("index.html").is_ok());
        let mut source = String::new();
        archive
            .by_name("reply.md")
            .unwrap()
            .read_to_string(&mut source)
            .unwrap();
        assert!(source.contains("media/"));
        assert!(archive.file_names().any(|name| name.starts_with("media/")));
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use std::io::Read;
    #[tokio::test]
    async fn explicit_export_uses_latest_snapshot_without_waiting_for_a_state_put() {
        let dir = tempfile::tempdir().unwrap();
        let store = RichStore::at(dir.path());
        let key = widget_state_key("reply/0", "<button>Counter</button>\n");
        let old = WidgetState {
            custom: serde_json::json!({"counter":1}),
            ..Default::default()
        };
        store.save_state(&key, &old).unwrap();
        let latest = WidgetState {
            custom: serde_json::json!({"counter":42}),
            ..Default::default()
        };
        let unrelated = WidgetState {
            custom: serde_json::json!({"private":"do not export another widget"}),
            ..Default::default()
        };
        let request = ExportRequest {
            markdown: "```bruhs-html\n<button>Counter</button>\n```".into(),
            scope: "reply".into(),
            state_keys: vec![key.clone()],
            include_state: true,
            widget_states: BTreeMap::from([
                (key, latest),
                (widget_state_key("other", "source"), unrelated),
            ]),
            ..Default::default()
        };
        let bundle = build(store, request).await.unwrap();
        let mut zip = zip::ZipArchive::new(bundle.reopen().unwrap()).unwrap();
        let mut html = String::new();
        zip.by_name("widgets/0.html")
            .unwrap()
            .read_to_string(&mut html)
            .unwrap();
        assert!(html.contains("\"counter\":42"));
        assert!(!html.contains("do not export another widget"));
    }
    #[tokio::test]
    async fn exports_never_embed_saved_values_without_explicit_opt_in() {
        let dir = tempfile::tempdir().unwrap();
        let key = widget_state_key("reply/0", "source");
        let state = WidgetState {
            custom: serde_json::json!({"private":"not opted in"}),
            ..Default::default()
        };
        let request = ExportRequest {
            markdown: "```bruhs-html\n<button>Widget</button>\n```".into(),
            state_keys: vec![key.clone()],
            widget_states: BTreeMap::from([(key, state)]),
            ..Default::default()
        };
        let bundle = build(RichStore::at(dir.path()), request).await.unwrap();
        let mut zip = zip::ZipArchive::new(bundle.reopen().unwrap()).unwrap();
        let mut html = String::new();
        zip.by_name("widgets/0.html")
            .unwrap()
            .read_to_string(&mut html)
            .unwrap();
        assert!(!html.contains("not opted in"));
    }
}
