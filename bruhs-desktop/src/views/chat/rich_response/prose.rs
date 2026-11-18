use super::*;
use bruhs_models::logic::rich_response::{
    all_media_sources, markdown_with_allowed_images, rewrite_media_paths,
};
use futures_util::StreamExt as _;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct ProseEntry {
    source: SharedString,
    sources: BTreeSet<String>,
    approved: BTreeSet<String>,
    paths: BTreeMap<String, String>,
    loading: bool,
    error: Option<String>,
    task: Option<Task<()>>,
    rendered: SharedString,
    dirty: bool,
}

impl ProseEntry {
    fn update(&mut self, source: &SharedString) {
        if self.source == *source {
            return;
        }
        self.sources = if source.contains('[') || source.contains("bruhs-") {
            all_media_sources(source)
        } else {
            BTreeSet::new()
        };
        self.approved.retain(|src| self.sources.contains(src));
        self.paths.retain(|src, _| self.sources.contains(src));
        self.source = source.clone();
        self.dirty = true;
    }

    fn approve(&mut self, sources: &[String]) {
        self.approved.extend(
            sources
                .iter()
                .filter(|src| self.sources.contains(*src))
                .cloned(),
        );
        self.dirty = true;
    }

    fn capture(&mut self, paths: BTreeMap<String, String>) {
        for (src, url) in paths {
            if self.sources.contains(&src) {
                self.paths.insert(src, url);
            }
        }
        self.dirty = true;
    }

    fn markdown(&mut self) -> SharedString {
        if !self.dirty {
            return self.rendered.clone();
        }
        let rewritten = if self.paths.is_empty() {
            self.source.to_string()
        } else {
            rewrite_media_paths(&self.source, &self.paths)
        };
        let mut allowed = self.approved.clone();
        allowed.extend(self.paths.values().cloned());
        self.rendered = markdown_with_allowed_images(&rewritten, &allowed).into();
        self.dirty = false;
        self.rendered.clone()
    }
}

impl ChatView {
    pub(in crate::views::chat) fn render_native_prose(
        &self,
        message: usize,
        block: usize,
        source: &SharedString,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let key = (message, block);
        let (markdown, remote, error, has_local) = {
            let mut embeds = self.rich_embeds.borrow_mut();
            let client = embeds.client.clone();
            let entry = embeds.prose.entry(key).or_default();
            entry.update(source);
            let remote = entry
                .sources
                .iter()
                .filter(|src| media_is_remote(src) && !entry.approved.contains(*src))
                .cloned()
                .collect::<Vec<_>>();
            let local = entry
                .sources
                .iter()
                .filter(|src| !media_is_remote(src) && !entry.paths.contains_key(*src))
                .cloned()
                .collect::<Vec<_>>();
            let has_local = !local.is_empty();
            if !local.is_empty() && !entry.loading && entry.error.is_none() {
                if let Some(client) = client {
                    entry.loading = true;
                    let project = self
                        .worktree_path
                        .as_deref()
                        .unwrap_or(&self.project_path)
                        .to_string();
                    let base = server_base();
                    let work = spawn_tokio(cx, async move {
                        let operations = local.into_iter().map(|src| {
                            let client = client.clone();
                            let project = project.clone();
                            let base = base.clone();
                            async move {
                                let asset = client
                                    .capture_rich_asset(&src, &project)
                                    .await
                                    .map_err(|error| error.to_string())?;
                                let url = if asset.signed_url.starts_with('/') {
                                    format!("{base}{}", asset.signed_url)
                                } else {
                                    asset.signed_url
                                };
                                Ok::<_, String>((src, url))
                            }
                        });
                        let mut operations = futures_util::stream::iter(operations)
                            .buffer_unordered(
                                bruhs_models::logic::attachments::MAX_FILES_PER_MESSAGE,
                            );
                        let mut paths = BTreeMap::new();
                        while let Some(result) =
                            futures_util::StreamExt::next(&mut operations).await
                        {
                            let (src, url) = result?;
                            paths.insert(src, url);
                        }
                        Ok::<_, String>(paths)
                    });
                    entry.task = Some(cx.spawn(async move |chat, cx| {
                        let result = work.await;
                        let _ = chat.update(cx, |this, cx| {
                            if let Some(entry) = this.rich_embeds.borrow_mut().prose.get_mut(&key) {
                                entry.loading = false;
                                match result {
                                    Ok(paths) => entry.capture(paths),
                                    Err(error) => entry.error = Some(error),
                                }
                            }
                            cx.notify();
                        });
                    }));
                }
            }
            (entry.markdown(), remote, entry.error.clone(), has_local)
        };
        if has_local {
            self.check_rich_response_host(cx);
        }
        let mut element = div()
            .debug_selector(move || format!("desktop-prose-{message}-{block}"))
            .min_w_0()
            .w_full();
        if !remote.is_empty() {
            element = element.child(
                runtime::action_button(
                    ("load-prose-images", block),
                    "Load external images",
                    cx,
                    move |this, _, cx| {
                        if let Some(entry) = this.rich_embeds.borrow_mut().prose.get_mut(&key) {
                            entry.approve(&remote);
                        }
                        cx.notify();
                    },
                )
                .debug_selector(move || format!("desktop-prose-images-{message}-{block}")),
            );
        }
        if let Some(error) = error {
            element = element.child(
                div()
                    .text_color(theme::lychee_400())
                    .child(format!("Could not keep these response images: {error}")),
            );
        }
        element
            .child(
                untrusted_markdown(
                    desktop_block_id("desktop-assistant-markdown", message, block),
                    markdown,
                )
                .style(desktop_markdown_style(cx))
                .selectable(true),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::ProseEntry;

    #[test]
    fn nested_images_keep_table_structure_and_require_exact_approval() {
        let mut entry = ProseEntry::default();
        entry.update(&"| Picture |\n| --- |\n| ![A](https://example.com/a.png) |".into());
        assert!(!entry.markdown().contains("![A]"));
        let sources = entry.sources.iter().cloned().collect::<Vec<_>>();
        entry.approve(&sources);
        assert!(entry.markdown().contains("![A]"));
        entry.update(&"| Picture |\n| --- |\n| ![A](https://example.com/a.png) ![B](https://example.com/b.png) |".into());
        assert!(entry.markdown().contains("![A]"));
        assert!(!entry.markdown().contains("![B]"));
    }

    #[test]
    fn local_list_images_use_only_captured_signed_resources() {
        let mut entry = ProseEntry::default();
        entry.update(&"- ![Diagram](./diagram.png)".into());
        assert!(!entry.markdown().contains("![Diagram]"));
        entry.paths.insert(
            "./diagram.png".into(),
            "http://127.0.0.1:3001/api/agent/rich-response/asset/token/diagram.png".into(),
        );
        entry.dirty = true;
        let markdown = entry.markdown();
        assert!(markdown.starts_with("- ![Diagram]"));
        assert!(markdown.contains("/asset/token/diagram.png"));
        assert!(!markdown.contains("(./diagram.png)"));
    }
}
