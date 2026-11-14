//! Rich blocks keep their own mounted state while later response text streams.

use bruhs_models::logic::rich_response::{
    media_frame_url, media_is_remote, media_url, response_blocks, HtmlBlock, MediaBlock, MediaKind,
    ResponseBlock,
};
use leptos::prelude::*;

use crate::api::BASE_URL;
use crate::markdown;
use crate::ui::button::{button_class, ButtonSize, ButtonVariant};

#[derive(Clone, Copy)]
pub struct ChatInteraction(pub Callback<()>);

mod bridge;
mod client;
mod diagnostics;
mod export;
mod media;
mod nested;
mod widget;

#[cfg(test)]
use media::InlineMedia;
pub(super) use widget::ArtifactResponse;
use widget::InteractiveResponse;

#[cfg(test)]
mod tests;

fn block_keys(blocks: &[ResponseBlock]) -> Vec<(usize, u8)> {
    blocks
        .iter()
        .enumerate()
        .map(|(index, block)| {
            let kind = match block {
                ResponseBlock::Markdown(_) => 0,
                ResponseBlock::Media(_) => 1,
                ResponseBlock::Html(_) => 2,
                ResponseBlock::Error { .. } => 3,
                ResponseBlock::Artifact(_) => 4,
            };
            (index, kind)
        })
        .collect()
}

#[component]
pub fn RichResponse(
    markdown: Signal<String>,
    streaming: Signal<bool>,
    project_path: Signal<Option<String>>,
    #[prop(default = String::new())] scope: String,
) -> impl IntoView {
    let blocks = Memo::new(move |_| response_blocks(&markdown.get(), streaming.get()));
    let keys = Memo::new(move |_| blocks.with(|blocks| block_keys(blocks)));
    let scope = StoredValue::new(scope);
    view! {
        <div class="min-w-0 px-1 py-0.5 pb-2" data-rich-response="">
            {move || (!streaming.get() && markdown.with(|text| text.trim().is_empty())).then(|| view! {
                <p class="text-body-sm text-rambutan-500">"(empty response)"</p>
            })}
            <For
                each=move || keys.get()
                key=|key| *key
                children=move |(index, _)| {
                    let block = Memo::new(move |_| blocks.with(|blocks| blocks.get(index).cloned()));
                    let block_scope = format!("{}/{index}", scope.get_value());
                    if matches!(block.get_untracked(), Some(ResponseBlock::Markdown(_))) {
                        let text = Signal::derive(move || block.with(|block| match block { Some(ResponseBlock::Markdown(text)) => text.clone(), _ => String::new() }));
                        view! { <nested::RichMarkdown markdown=text project_path scope=block_scope /> }.into_any()
                    } else if matches!(block.get_untracked(),Some(ResponseBlock::Media(_))) {
                        let media=Signal::derive(move ||block.with(|block|match block {Some(ResponseBlock::Media(media))=>media.clone(),_=>MediaBlock::default()}));
                        view! { <media::MediaResponse media project_path /> }.into_any()
                    } else {
                        let block_scope = StoredValue::new(block_scope);
                        view! { {move || block.get().map(|block| render_block(block, project_path, block_scope.get_value()))} }.into_any()
                    }
                }
            />
            <export::ExportResponse markdown project_path scope=scope.get_value() streaming />
        </div>
    }
}

fn render_block(
    block: ResponseBlock,
    project_path: Signal<Option<String>>,
    scope: String,
) -> AnyView {
    match block {
        ResponseBlock::Markdown(text) => view! { <nested::RichMarkdown markdown=Signal::stored(text) project_path scope /> }.into_any(),
        ResponseBlock::Media(media) => view! { <media::MediaResponse media=Signal::stored(media) project_path /> }.into_any(),
        ResponseBlock::Html(html) => view! { <InteractiveResponse html scope /> }.into_any(),
        ResponseBlock::Artifact(artifact) => view! { <ArtifactResponse artifact scope /> }.into_any(),
        ResponseBlock::Error { source, message } => view! {
            <div class="my-2 min-w-0 text-body-sm text-rambutan-400" data-rich-response-error="">
                <p role="status" class="text-lychee-300">{message}</p>
                <details class="mt-1">
                    <summary class="bruhs-control min-h-7 cursor-pointer py-1 text-label-sm">"Show source"</summary>
                    <pre class="max-h-80 overflow-auto whitespace-pre-wrap break-words rounded-grape bg-longan-900 p-3 text-code-sm"><code>{source}</code></pre>
                </details>
            </div>
        }.into_any(),
    }
}
