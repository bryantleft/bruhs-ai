use bruhs_models::logic::{
    attachments::{kind_for_name, FileKind},
    rich_response::{validate_media_source, widget_state_key},
};
use leptos::html::Div;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Slot {
    id: String,
    marker: String,
    media: MediaBlock,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Prepared {
    markdown: String,
    slots: Vec<Slot>,
}

fn prepare(markdown: &str, prefix: &str) -> Prepared {
    let mut events = Parser::new_ext(markdown, Options::all()).into_offset_iter();
    let mut replacements = Vec::new();
    while let Some((event, range)) = events.next() {
        let (kind, source, mut title, end) = match event {
            Event::Start(Tag::Image {
                dest_url, title, ..
            }) => (
                Some(MediaKind::Image),
                dest_url.into_string(),
                title.into_string(),
                TagEnd::Image,
            ),
            Event::Start(Tag::Link {
                dest_url, title, ..
            }) => {
                let path = dest_url.split(['?', '#']).next().unwrap_or(&dest_url);
                let kind = match kind_for_name(path) {
                    FileKind::Image => Some(MediaKind::Image),
                    FileKind::Video => Some(MediaKind::Video),
                    FileKind::Audio => Some(MediaKind::Audio),
                    FileKind::Pdf => Some(MediaKind::Pdf),
                    _ => None,
                };
                (
                    kind,
                    dest_url.into_string(),
                    title.into_string(),
                    TagEnd::Link,
                )
            }
            _ => continue,
        };
        let mut image = None;
        let mut alt = String::new();
        for (event, _) in events.by_ref() {
            match event {
                Event::End(tag) if tag == end => break,
                Event::Start(Tag::Image { dest_url, .. }) => image = Some(dest_url.into_string()),
                Event::Text(text) | Event::Code(text) => alt.push_str(&text),
                _ => {}
            }
        }
        let (kind, source) = if let Some(source) = image {
            (MediaKind::Image, source)
        } else if let Some(kind) = kind {
            (kind, source)
        } else {
            continue;
        };
        if validate_media_source(&source).is_err() {
            continue;
        }
        if !alt.is_empty() {
            title = alt;
        }
        let media = MediaBlock {
            kind,
            src: source,
            title,
            ..Default::default()
        };
        let id = prefix.to_string();
        let marker = String::new();
        replacements.push((range, Slot { id, marker, media }));
    }
    let mut rendered = String::with_capacity(markdown.len());
    let mut cursor = 0;
    let mut slots = Vec::new();
    for (range, mut slot) in replacements {
        if range.start < cursor {
            continue;
        }
        // Identical media can appear in several cells without sharing one DOM mount.
        slot.id.push_str(&format!("-{}", slots.len()));
        slot.marker = format!("\u{e000}{}\u{e001}", slot.id);
        rendered.push_str(&markdown[cursor..range.start]);
        rendered.push_str(&slot.marker);
        cursor = range.end;
        slots.push(slot);
    }
    rendered.push_str(&markdown[cursor..]);
    Prepared {
        markdown: rendered,
        slots,
    }
}

#[component]
pub(super) fn RichMarkdown(
    markdown: Signal<String>,
    project_path: Signal<Option<String>>,
    scope: String,
) -> impl IntoView {
    let root = NodeRef::<Div>::new();
    let prefix = StoredValue::new(format!(
        "{}-{}",
        widget_state_key("prose", &scope),
        uuid::Uuid::new_v4()
    ));
    let prepared = Memo::new(move |_| prepare(&markdown.get(), &prefix.get_value()));
    let html = move || {
        prepared.with(|prepared| {
            let slots = prepared
                .slots
                .iter()
                .map(|slot| (slot.marker.as_str(), slot.id.as_str()))
                .collect::<Vec<_>>();
            markdown::dom::render_slots(&prepared.markdown, &slots)
        })
    };
    let prose = leptos::tachys::html::element::InnerHtmlAttribute::inner_html(
        leptos::html::div()
            .node_ref(root)
            .class("prose-msg text-body-sm leading-relaxed text-rambutan-200"),
        html,
    );
    view! {
        {prose}
        <For each=move || prepared.with(|prepared| prepared.slots.iter().map(|slot|slot.id.clone()).collect::<Vec<_>>()) key=|id|id.clone() children=move |id| {
            let media_slot=Memo::new(move |_|prepared.with(|prepared|prepared.slots.iter().find(|slot|slot.id==id).cloned()));
            view! { <MountedMedia root markdown media_slot project_path /> }
        } />
    }
}

#[component]
fn MountedMedia(
    root: NodeRef<Div>,
    markdown: Signal<String>,
    media_slot: Memo<Option<Slot>>,
    project_path: Signal<Option<String>>,
) -> impl IntoView {
    let target = RwSignal::new_local(None::<web_sys::Element>);
    let slot = StoredValue::new(media_slot.get_untracked());
    let media = Signal::derive(move || {
        media_slot
            .with(|slot| slot.as_ref().map(|slot| slot.media.clone()))
            .or_else(|| slot.get_value().map(|slot| slot.media))
            .unwrap_or_default()
    });
    Effect::new(move |_| {
        markdown.track();
        let Some(root) = root.get() else {
            return;
        };
        let Some(slot) = slot.get_value() else {
            return;
        };
        let id = slot.id;
        gloo_timers::callback::Timeout::new(0, move || {
            let Ok(Some(element)) =
                root.query_selector(&format!("[data-rich-media-slot=\"{id}\"]"))
            else {
                return;
            };
            let unchanged = target
                .try_with_untracked(|target| {
                    target
                        .as_ref()
                        .is_some_and(|old| old.is_same_node(Some(&element)))
                })
                .unwrap_or(false);
            if !unchanged {
                let _ = target.try_set(Some(element));
            }
        })
        .forget();
    });
    view! {
        {move || target.get().map(|mount| view! { <leptos::portal::Portal mount><media::MediaResponse media project_path /></leptos::portal::Portal> })}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn immutable_source_rewrites_keep_nested_mount_identity() {
        let before = prepare("- ![image](output/plot.png)", "scope");
        let after = prepare("- ![image](/immutable/plot.png)", "scope");
        assert_eq!(before.slots[0].id, after.slots[0].id);
        assert_ne!(before.slots[0].media.src, after.slots[0].media.src);
    }
    #[test]
    fn nested_media_keeps_table_list_and_emphasis_markup() {
        let input = "| A | B |\n|---|---|\n| ![plot](plot.png) | **![remote](https://example.com/a.png)** |\n\n- [clip](clip.mp4)\n- [image file](chart.png)\n- [ordinary page](https://example.com/page)\n\n`![code](example.png)`";
        let prepared = prepare(input, "test");
        assert_eq!(prepared.slots.len(), 4);
        assert!(prepared.markdown.contains("|---|---|"));
        assert!(prepared.markdown.contains("**\u{e000}"));
        assert!(prepared.markdown.contains("- \u{e000}"));
        assert!(prepared.markdown.contains("`![code](example.png)`"));
        assert!(prepared
            .markdown
            .contains("[ordinary page](https://example.com/page)"));
    }
}
