use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

fn render_media(kind: MediaKind, src: &str) -> String {
    let _ = any_spawner::Executor::init_futures_executor();
    let owner = Owner::new();
    owner.with(|| {
        untrack(|| {
            tachys::view::RenderHtml::to_html(view! {
                <InlineMedia
                    media=MediaBlock { kind, src: src.into(), title: "A response".into(), poster: None, tracks: Vec::new(), transcript: None }
                    project_path=Signal::stored(Some("/project".into()))
                />
            })
        })
    })
}

#[test]
fn remote_media_has_no_fetching_element_before_the_user_loads_it() {
    for kind in [
        MediaKind::Image,
        MediaKind::Video,
        MediaKind::Audio,
        MediaKind::Pdf,
    ] {
        let html = render_media(kind, "https://example.com/media");
        assert!(html.contains("Load "));
        for tag in ["<img", "<video", "<audio", "<iframe"] {
            assert!(
                !html.contains(tag),
                "{kind:?} fetched through {tag}: {html}"
            );
        }
    }
}

#[test]
fn local_playback_uses_native_controls_without_autoplay_or_eager_loading() {
    for (kind, tag) in [(MediaKind::Video, "video"), (MediaKind::Audio, "audio")] {
        let html = render_media(kind, "output/clip.mp4");
        assert!(html.contains(&format!("<{tag}")), "{html}");
        assert!(html.contains("controls"), "{html}");
        assert!(html.contains("preload=\"none\""), "{html}");
        assert!(!html.contains("autoplay"), "{html}");
        assert!(html.contains("/api/agent/media?path="), "{html}");
        assert!(html.contains("project="), "{html}");
    }
}

#[test]
fn streamed_prose_keeps_the_rich_block_key_and_memo_unchanged() {
    let owner = Owner::new();
    owner.with(|| {
        let html = ResponseBlock::Html(HtmlBlock {
            html: "<input value=\"kept\">".into(),
            title: String::new(),
        });
        let source = RwSignal::new(vec![html.clone(), ResponseBlock::Markdown("tail".into())]);
        let keys = Memo::new(move |_| source.with(|blocks| block_keys(blocks)));
        let rich = Memo::new(move |_| source.with(|blocks| blocks[0].clone()));
        let renders = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&renders);
        let rendered = Memo::new(move |_| {
            count.fetch_add(1, Ordering::Relaxed);
            rich.get()
        });
        let original = keys.get_untracked();
        assert_eq!(rendered.get_untracked(), html);
        source.update(|blocks| blocks[1] = ResponseBlock::Markdown("tail plus tokens".into()));
        assert_eq!(keys.get_untracked(), original);
        assert_eq!(rendered.get_untracked(), html);
        assert_eq!(renders.load(Ordering::Relaxed), 1);
    });
}

#[test]
fn interactive_content_is_an_opaque_frame_and_never_host_html() {
    let _ = any_spawner::Executor::init_futures_executor();
    let owner = Owner::new();
    let html = owner.with(|| {
        untrack(|| {
            tachys::view::RenderHtml::to_html(view! {
                <InteractiveResponse html=HtmlBlock {
                    html: "<script>parent.document.body.textContent='unsafe'</script>".into(),
                    title: "A chart".into(),
                } />
            })
        })
    });
    assert!(html.contains("sandbox=\"allow-scripts\""), "{html}");
    assert!(!html.contains("allow-same-origin"), "{html}");
    assert!(html.contains("/api/agent/rich-response#"), "{html}");
    assert!(!html.contains("<script>"), "{html}");
    assert!(html.contains("Reload interactive response"), "{html}");
}
