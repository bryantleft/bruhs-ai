//! Native Knowledge surface matching the web library's workflows and Bruhs
//! hierarchy. Document and source mutations share the desktop's one atomic
//! state snapshot; attachment payloads are copied beside that snapshot.

use std::cell::RefCell;
use std::collections::BTreeSet;

use bruhs_models::logic::knowledge_library::{self as library, document_copy, source_host, Card};
use bruhs_models::logic::rich_response::markdown_with_allowed_images;
use bruhs_models::num::ToFloatLossy;
use gpui::prelude::*;
use gpui::*;
use gpui_component::input::{Input, InputEvent};
use gpui_component::text::TextView;

use crate::views::chat::desktop_markdown_style;

use super::knowledge_storage::{self, attachment_path as knowledge_attachment_path};
use super::library_pages_ui::LibraryPage;
use super::record_picker_ui::RecordPickerTarget;
use super::*;

// The web knowledge library's 24-card budget. The native action fixture
// measures libraries through 10,000 records; paging never truncates stored text.
pub(super) const KNOWLEDGE_PAGE_SIZE: usize = library::PAGE_SIZE;

/// The library's search box and the shared index it reads (web
/// `KnowledgeLibrary`'s query and memos), and the open document's preview.
pub(super) struct KnowledgeState {
    pub(super) search_input: Entity<InputState>,
    /// Rebuilt when a document or source is added or removed, or when the
    /// workspace saves an edit; never on a keystroke.
    index: RefCell<Option<LibraryIndex>>,
    /// The open document shows its rendered Markdown instead of the editor
    /// (web `DocumentEditor`'s Preview). Each document opens in the editor.
    pub(super) preview: bool,
}

struct LibraryIndex {
    counts: (usize, usize),
    documents: library::Index,
    sources: library::Index,
}

/// One library section's page of cards and how many records match.
struct LibrarySection {
    cards: Vec<Card>,
    matches: usize,
}

impl KnowledgeState {
    pub(super) fn new(
        window: &mut Window,
        cx: &mut Context<'_, Workspace>,
        subscriptions: &mut Vec<Subscription>,
    ) -> Self {
        let search_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search documents, tags, clips, and sources…")
        });
        // Web: a new query starts both sections on their first page.
        subscriptions.push(cx.subscribe_in(
            &search_input,
            window,
            |this: &mut Workspace, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.library_pages.documents = 0;
                    this.library_pages.sources = 0;
                    cx.notify();
                }
            },
        ));
        Self {
            search_input,
            index: RefCell::new(None),
            preview: false,
        }
    }

    /// Drop the index so the next render searches the saved edit.
    pub(super) fn invalidate(&mut self) {
        self.index.get_mut().take();
    }
}

/// Web's document body field: `min-h-[384px]`.
const DOCUMENT_BODY_HEIGHT_PX: f32 = 384.0;
/// Web's source excerpt field: 12 rows.
const SOURCE_EXCERPT_HEIGHT_PX: f32 = 264.0;

/// The web Knowledge page's subtitle.
const KNOWLEDGE_PAGE_SUBTITLE: &str = "Markdown documents and saved sources, local and searchable.";

impl Workspace {
    /// Run `read` over the documents' and sources' shared search indexes.
    fn with_knowledge_index<R>(
        &self,
        read: impl FnOnce(&library::Index, &library::Index) -> R,
    ) -> R {
        let counts = (self.knowledge_documents.len(), self.source_clips.len());
        let mut cache = self.knowledge.index.borrow_mut();
        if cache.as_ref().is_some_and(|index| index.counts != counts) {
            cache.take();
        }
        let index = cache.get_or_insert_with(|| LibraryIndex {
            counts,
            documents: library::Index::documents(&self.knowledge_documents),
            sources: library::Index::sources(&self.source_clips),
        });
        read(&index.documents, &index.sources)
    }

    fn knowledge_query(&self, cx: &App) -> String {
        self.knowledge.search_input.read(cx).value().to_string()
    }

    /// The page of the library section that shows `id` under the current
    /// search, when it matches.
    fn knowledge_page_of(&self, target: LibraryPage, id: &str, cx: &App) -> Option<usize> {
        let query = self.knowledge_query(cx);
        self.with_knowledge_index(|documents, sources| {
            let index = match target {
                LibraryPage::Sources => sources,
                _ => documents,
            };
            index.page_of(&index.matching(&query), id)
        })
    }

    pub(super) fn open_knowledge_view(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.active_view = AppView::Knowledge;
        self.clip_create_visible = false;
        self.compact_sidebar_open = false;
        self.close_transient_surfaces();
        self.workspace_focus_handle.focus(window, cx);
        self.persist_and_notify(cx);
    }

    pub(super) fn new_knowledge_document(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let document = KnowledgeDocument::new("Untitled", "", EpochMs::now().as_millis());
        let id = document.id.clone();
        self.knowledge_documents.insert(0, document);
        self.select_knowledge_document(&id, window, cx);
    }

    pub(super) fn select_knowledge_document(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(page) = self.knowledge_page_of(LibraryPage::Documents, id, cx) {
            self.library_pages.select(LibraryPage::Documents, page);
        }
        let Some(document) = self
            .knowledge_documents
            .iter()
            .find(|document| document.id == id)
            .cloned()
        else {
            return;
        };
        self.knowledge_title_input.update(cx, |input, cx| {
            input.set_value(document.title, window, cx);
        });
        self.knowledge_body_input.update(cx, |input, cx| {
            input.set_value(document.body_markdown, window, cx);
        });
        self.knowledge_tags_input.update(cx, |input, cx| {
            input.set_value(document.tags.join(", "), window, cx);
        });
        self.active_view = AppView::DocumentDetail {
            document_id: id.to_string(),
        };
        self.knowledge.preview = false;
        self.clip_create_visible = false;
        self.persist_and_notify(cx);
        self.knowledge_title_input.update(cx, |input, cx| {
            input.focus(window, cx);
        });
    }

    /// Web `DocumentEditor`'s Preview and Edit: show the body's rendered
    /// Markdown, including unsaved changes, or the editor again.
    pub(super) fn toggle_knowledge_preview(&mut self, cx: &mut Context<'_, Self>) {
        self.knowledge.preview = !self.knowledge.preview;
        cx.notify();
    }

    pub(super) fn start_source_clip(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        for input in [
            &self.clip_title_input,
            &self.clip_url_input,
            &self.clip_source_name_input,
            &self.clip_author_input,
            &self.clip_published_input,
            &self.clip_tags_input,
            &self.clip_excerpt_input,
        ] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.active_view = AppView::Knowledge;
        self.clip_create_visible = true;
        cx.notify();
        self.clip_title_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    pub(super) fn select_source_clip(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(page) = self.knowledge_page_of(LibraryPage::Sources, id, cx) {
            self.library_pages.select(LibraryPage::Sources, page);
        }
        let Some(clip) = self.source_clips.iter().find(|clip| clip.id == id).cloned() else {
            return;
        };
        self.clip_title_input
            .update(cx, |input, cx| input.set_value(clip.title, window, cx));
        self.clip_url_input.update(cx, |input, cx| {
            input.set_value(clip.source_url, window, cx);
        });
        self.clip_source_name_input.update(cx, |input, cx| {
            input.set_value(clip.source_name, window, cx);
        });
        self.clip_author_input.update(cx, |input, cx| {
            input.set_value(clip.author, window, cx);
        });
        self.clip_published_input.update(cx, |input, cx| {
            input.set_value(clip.published_on.unwrap_or_default(), window, cx);
        });
        self.clip_tags_input.update(cx, |input, cx| {
            input.set_value(clip.tags.join(", "), window, cx);
        });
        self.clip_excerpt_input.update(cx, |input, cx| {
            input.set_value(clip.excerpt, window, cx);
        });
        self.active_view = AppView::ClipDetail {
            clip_id: id.to_string(),
        };
        self.clip_create_visible = false;
        self.persist_and_notify(cx);
    }

    /// The open document's fields differ from what is saved (web `dirty`).
    pub(super) fn current_document_dirty(&self, cx: &App) -> bool {
        let AppView::DocumentDetail { document_id } = &self.active_view else {
            return false;
        };
        let Some(document) = self
            .knowledge_documents
            .iter()
            .find(|document| document.id == *document_id)
        else {
            return false;
        };
        let tags = self
            .knowledge_tags_input
            .read(cx)
            .value()
            .split(',')
            .map(str::to_string)
            .collect();
        document.clone().revise(
            self.knowledge_title_input.read(cx).value().to_string(),
            self.knowledge_body_input.read(cx).value().to_string(),
            tags,
            document.updated_at_ms,
        )
    }

    /// Web's line under the tags: whether the document is saved.
    fn document_save_line(&self, cx: &App) -> &'static str {
        match self.save_status {
            SaveStatus::Saving => document_copy::SAVING,
            SaveStatus::Failed(_) | SaveStatus::Conflict | SaveStatus::RecoveryRequired(_) => {
                document_copy::NOT_SAVED
            }
            SaveStatus::Saved if self.current_document_dirty(cx) => document_copy::UNSAVED,
            SaveStatus::Saved => document_copy::SAVED,
        }
    }

    pub(super) fn save_current_document(&mut self, cx: &mut Context<'_, Self>) {
        let AppView::DocumentDetail { document_id } = &self.active_view else {
            return;
        };
        let title = self.knowledge_title_input.read(cx).value().to_string();
        let body = self.knowledge_body_input.read(cx).value().to_string();
        let tags = self
            .knowledge_tags_input
            .read(cx)
            .value()
            .split(',')
            .map(str::to_string)
            .collect();
        if let Some(document) = self
            .knowledge_documents
            .iter_mut()
            .find(|document| document.id == *document_id)
        {
            document.revise(title, body, tags, EpochMs::now().as_millis());
            self.persist_and_notify(cx);
        }
    }

    fn save_source_clip(&mut self, cx: &mut Context<'_, Self>) {
        let title = self.clip_title_input.read(cx).value().trim().to_string();
        let url = self.clip_url_input.read(cx).value().trim().to_string();
        if title.is_empty() || url.is_empty() {
            self.persistence_error = Some("A source title and URL are required.".to_string());
            cx.notify();
            return;
        }
        let excerpt = self.clip_excerpt_input.read(cx).value().to_string();
        let source_name = self
            .clip_source_name_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let author = self.clip_author_input.read(cx).value().trim().to_string();
        let published_on = self
            .clip_published_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let tags = bruhs_models::normalize_tags(
            self.clip_tags_input
                .read(cx)
                .value()
                .split(',')
                .map(str::to_string)
                .collect(),
        );
        match &self.active_view {
            AppView::ClipDetail { clip_id } => {
                if let Some(clip) = self
                    .source_clips
                    .iter_mut()
                    .find(|clip| clip.id == *clip_id)
                {
                    clip.title = title;
                    clip.source_url = url;
                    clip.source_name = source_name;
                    clip.author = author;
                    clip.published_on = (!published_on.is_empty()).then_some(published_on);
                    clip.tags = tags;
                    clip.excerpt = excerpt;
                    clip.updated_at_ms = EpochMs::now().as_millis();
                }
            }
            _ => {
                let mut clip = SourceClip::new(title, url, excerpt, EpochMs::now().as_millis());
                clip.source_name = source_name;
                clip.author = author;
                clip.published_on = (!published_on.is_empty()).then_some(published_on);
                clip.tags = tags;
                self.active_view = AppView::ClipDetail {
                    clip_id: clip.id.clone(),
                };
                self.source_clips.insert(0, clip);
                self.library_pages.sources = 0;
                self.clip_create_visible = false;
            }
        }
        self.persistence_error = None;
        self.persist_and_notify(cx);
    }

    pub(super) fn delete_current_document(&mut self, cx: &mut Context<'_, Self>) {
        let AppView::DocumentDetail { document_id } = &self.active_view else {
            return;
        };
        let removed = self
            .knowledge_documents
            .iter()
            .find(|document| document.id == *document_id)
            .cloned();
        self.knowledge_documents
            .retain(|document| document.id != *document_id);
        if let Some(document) = removed {
            let ids: Vec<String> = document
                .attachments
                .into_iter()
                .map(|attachment| attachment.id)
                .collect();
            if let Some(dir) = crate::paths::knowledge_attachments_dir() {
                if !ids.is_empty() {
                    cx.background_spawn(async move {
                        knowledge_storage::remove_attachments(&dir, &ids);
                    })
                    .detach();
                }
            }
        }
        self.active_view = AppView::Knowledge;
        self.persist_and_notify(cx);
    }

    pub(super) fn delete_current_source_clip(&mut self, cx: &mut Context<'_, Self>) {
        let AppView::ClipDetail { clip_id } = &self.active_view else {
            return;
        };
        let clip_id = clip_id.clone();
        self.source_clips.retain(|clip| clip.id != clip_id);
        for document in &mut self.knowledge_documents {
            if document
                .citations
                .iter()
                .any(|citation| citation.clip_id == clip_id)
            {
                let citations = document
                    .citations
                    .iter()
                    .filter(|citation| citation.clip_id != clip_id)
                    .cloned()
                    .collect();
                document.update_citations(citations, EpochMs::now().as_millis());
            }
        }
        self.active_view = AppView::Knowledge;
        self.persist_and_notify(cx);
    }

    fn ask_agent_about_current_document(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let AppView::DocumentDetail { document_id } = &self.active_view else {
            return;
        };
        let Some(document) = self
            .knowledge_documents
            .iter()
            .find(|document| document.id == *document_id)
            .cloned()
        else {
            return;
        };
        let prompt = bruhs_models::document_agent_prompt(&document, &self.source_clips);
        let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) else {
            return;
        };
        self.active_view = AppView::Chat;
        chat.update(cx, |chat, cx| {
            chat.set_composer_draft(prompt, window, cx);
        });
        self.persist_and_notify(cx);
    }

    fn ask_agent_about_current_source(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let AppView::ClipDetail { clip_id } = &self.active_view else {
            return;
        };
        let Some(clip) = self.source_clips.iter().find(|clip| clip.id == *clip_id) else {
            return;
        };
        let prompt = bruhs_models::source_clip_agent_prompt(clip);
        let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) else {
            return;
        };
        self.active_view = AppView::Chat;
        chat.update(cx, |chat, cx| {
            chat.set_composer_draft(prompt, window, cx);
        });
        self.persist_and_notify(cx);
    }

    /// Connect `link` to the open document, once (web `add_link`).
    pub(super) fn add_knowledge_link(&mut self, link: KnowledgeLink, cx: &mut Context<'_, Self>) {
        let AppView::DocumentDetail { document_id } = &self.active_view else {
            return;
        };
        let Some(document) = self
            .knowledge_documents
            .iter_mut()
            .find(|document| document.id == *document_id)
        else {
            return;
        };
        if document
            .links
            .iter()
            .any(|item| item.kind == link.kind && item.target_id == link.target_id)
        {
            return;
        }
        let mut links = document.links.clone();
        links.push(link);
        document.update_links(links, EpochMs::now().as_millis());
        self.persist_and_notify(cx);
    }

    /// The records a document can connect to, in the web's order: other
    /// documents, tasks, projects, meetings, then people.
    pub(super) fn knowledge_link_candidates(
        &self,
        current_id: &str,
        cx: &App,
    ) -> Vec<KnowledgeLink> {
        self.knowledge_documents
            .iter()
            .filter(|document| document.id != current_id)
            .map(|document| KnowledgeLink {
                kind: KnowledgeLinkKind::Document,
                target_id: document.id.clone(),
                label: document.title.clone(),
            })
            .chain(self.work_items.iter().map(|item| KnowledgeLink {
                kind: KnowledgeLinkKind::Task,
                target_id: item.id.clone(),
                label: item.title.clone(),
            }))
            .chain(self.projects.iter().map(|project| KnowledgeLink {
                kind: KnowledgeLinkKind::Project,
                target_id: project.id.clone(),
                label: project.name.clone(),
            }))
            .chain(
                self.meetings_hub
                    .read(cx)
                    .meetings
                    .iter()
                    .map(|meeting| KnowledgeLink {
                        kind: KnowledgeLinkKind::Meeting,
                        target_id: meeting.id.clone(),
                        label: meeting.title.clone(),
                    }),
            )
            .chain(
                self.meetings_hub
                    .read(cx)
                    .people
                    .iter()
                    .map(|person| KnowledgeLink {
                        kind: KnowledgeLinkKind::Person,
                        target_id: person.id.clone(),
                        label: person.display_name.clone(),
                    }),
            )
            .collect()
    }

    fn remove_knowledge_link(
        &mut self,
        document_id: &str,
        index: usize,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(document) = self
            .knowledge_documents
            .iter_mut()
            .find(|document| document.id == document_id)
        {
            let mut links = document.links.clone();
            if index < links.len() {
                links.remove(index);
                document.update_links(links, EpochMs::now().as_millis());
                self.persist_and_notify(cx);
            }
        }
    }

    /// Cite the saved source `clip_id` in the open document, once (web
    /// `add_citation`).
    pub(super) fn add_citation(&mut self, clip_id: &str, cx: &mut Context<'_, Self>) {
        let AppView::DocumentDetail { document_id } = &self.active_view else {
            return;
        };
        let Some(document) = self
            .knowledge_documents
            .iter_mut()
            .find(|document| document.id == *document_id)
        else {
            return;
        };
        if document
            .citations
            .iter()
            .any(|citation| citation.clip_id == clip_id)
        {
            return;
        }
        let mut citations = document.citations.clone();
        citations.push(KnowledgeCitation {
            clip_id: clip_id.to_string(),
            locator: String::new(),
            note: String::new(),
        });
        document.update_citations(citations, EpochMs::now().as_millis());
        self.persist_and_notify(cx);
    }

    fn remove_citation(&mut self, document_id: &str, clip_id: &str, cx: &mut Context<'_, Self>) {
        if let Some(document) = self
            .knowledge_documents
            .iter_mut()
            .find(|document| document.id == document_id)
        {
            let mut citations = document.citations.clone();
            citations.retain(|citation| citation.clip_id != clip_id);
            document.update_citations(citations, EpochMs::now().as_millis());
            self.persist_and_notify(cx);
        }
    }

    fn restore_document_version(
        &mut self,
        document_id: &str,
        version_id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(document) = self
            .knowledge_documents
            .iter_mut()
            .find(|document| document.id == document_id)
        {
            if document.restore_version(version_id, EpochMs::now().as_millis()) {
                let repair = self
                    .meetings_hub
                    .read(cx)
                    .canonicalize_person_links(&mut document.links);
                let restored = document.clone();
                self.knowledge_title_input.update(cx, |input, cx| {
                    input.set_value(restored.title, window, cx);
                });
                self.knowledge_body_input.update(cx, |input, cx| {
                    input.set_value(restored.body_markdown, window, cx);
                });
                self.knowledge_tags_input.update(cx, |input, cx| {
                    input.set_value(restored.tags.join(", "), window, cx);
                });
                if let Err(error) = repair {
                    self.meetings_hub.update(cx, |hub, cx| {
                        for link in &restored.links {
                            if link.kind == KnowledgeLinkKind::Person
                                && hub.person_is_retired(&link.target_id)
                            {
                                hub.wait_for_person_merge(link.target_id.clone());
                            }
                        }
                        hub.report_person_merge_conflict(error.into(), cx);
                    });
                }
                self.persist_and_notify(cx);
            }
        }
    }

    fn attach_to_current_document(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let AppView::DocumentDetail { document_id } = &self.active_view else {
            return;
        };
        let document_id = document_id.clone();
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Attach a local file".into()),
        });
        let workspace = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let path = match picker.await {
                    Ok(Ok(Some(mut paths))) => paths.pop(),
                    _ => None,
                };
                let Some(path) = path else {
                    return;
                };
                let file_name = path.file_name().map_or_else(
                    || "Attachment".to_string(),
                    |name| name.to_string_lossy().to_string(),
                );
                let id = knowledge_storage::new_attachment_id(&path);
                let mime_type = mime_guess_from_name(&file_name).to_string();
                let attachment_id = id.clone();
                let result = cx
                    .background_spawn(async move {
                        let dir = crate::paths::knowledge_attachments_dir()
                            .ok_or(knowledge_storage::AttachmentError::NoStorage)?;
                        knowledge_storage::store_attachment(&path, &dir, &attachment_id)
                    })
                    .await;
                let size_bytes = match result {
                    Ok(size) => size,
                    Err(error) => {
                        tracing::warn!("attaching a file failed: {error:?}");
                        let _ = workspace.update(cx, |this, cx| {
                            this.persistence_error = Some(error.to_string());
                            cx.notify();
                        });
                        return;
                    }
                };
                let _ = workspace.update(cx, |this, cx| {
                    if let Some(document) = this
                        .knowledge_documents
                        .iter_mut()
                        .find(|document| document.id == document_id)
                    {
                        let mut attachments = document.attachments.clone();
                        attachments.push(bruhs_models::AttachmentMeta {
                            id,
                            file_name,
                            mime_type,
                            size_bytes,
                        });
                        document.update_attachments(attachments, EpochMs::now().as_millis());
                        this.persist_and_notify(cx);
                    }
                });
            })
            .detach();
    }

    pub(super) fn render_knowledge_page(&self, cx: &Context<'_, Self>) -> Div {
        if self.clip_create_visible {
            return self.render_source_clip_editor(true, cx);
        }
        let query = self.knowledge_query(cx);
        let (documents, sources) = self.with_knowledge_index(|documents, sources| {
            let section = |index: &library::Index, page| {
                let matches = index.matching(&query);
                LibrarySection {
                    cards: index.page(&matches, page),
                    matches: matches.len(),
                }
            };
            (
                section(documents, self.library_pages.documents),
                section(sources, self.library_pages.sources),
            )
        });
        let document_count = documents.matches;
        let source_count = sources.matches;
        let document_cards = documents
            .cards
            .iter()
            .map(|card| self.render_document_card(card, cx))
            .collect::<Vec<_>>();
        let source_cards = sources
            .cards
            .iter()
            .map(|card| self.render_source_card(card, cx))
            .collect::<Vec<_>>();
        div()
            .debug_selector(|| "desktop-knowledge-page".to_string())
            .flex_1()
            .h_full()
            .min_w_0()
            .bg(cx.theme().background)
            .flex()
            .flex_col()
            .child(
                div()
                    .px_5()
                    .py_3()
                    .border_b_1()
                    .border_color(theme::longan_800().opacity(0.5))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .child(
                                div()
                                    .font_family(theme::HEADING_FONT_FAMILY)
                                    .text_size(px(16.0))
                                    .text_color(theme::rambutan_100())
                                    .child("Knowledge"),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(theme::rambutan_500())
                                    .child(KNOWLEDGE_PAGE_SUBTITLE),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                web_button(
                                    "new-source-clip",
                                    WebButtonVariant::Ghost,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .debug_selector(|| "desktop-knowledge-new-clip".to_string())
                                .aria_label("Web clip")
                                .child("Web clip")
                                .on_click(cx.listener(
                                    |this, _event, window, cx| {
                                        this.start_source_clip(window, cx);
                                    },
                                )),
                            )
                            .child(
                                web_button(
                                    "new-knowledge-document",
                                    WebButtonVariant::Primary,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .debug_selector(|| "desktop-knowledge-new-document".to_string())
                                .aria_label("New document")
                                .child("New document")
                                .on_click(cx.listener(
                                    |this, _event, window, cx| {
                                        this.new_knowledge_document(window, cx);
                                    },
                                )),
                            ),
                    ),
            )
            .child(
                div()
                    .id("knowledge-library-scroll")
                    .flex_1()
                    .min_h_0()
                    .track_scroll(&self.library_pages.knowledge_scroll)
                    .overflow_y_scroll()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_5()
                    .items_center()
                    // Web: the search box opens the scrolling content.
                    .child(
                        div()
                            .debug_selector(|| "desktop-knowledge-search".to_string())
                            .w_full()
                            .max_w(px(980.0))
                            .flex_shrink_0()
                            .child(
                                Input::new(&self.knowledge.search_input)
                                    .aria_label("Search knowledge"),
                            ),
                    )
                    .child(
                        knowledge_section(
                            "Documents",
                            document_count,
                            document_cards,
                            "No documents match yet.",
                        )
                        .w_full()
                        .max_w(px(980.0))
                        .flex_shrink_0()
                        .children(self.render_library_pages(
                            LibraryPage::Documents,
                            document_count,
                            KNOWLEDGE_PAGE_SIZE,
                            "Documents",
                            cx,
                        )),
                    )
                    .child(
                        knowledge_section(
                            "Sources",
                            source_count,
                            source_cards,
                            "Save a web source to cite it later.",
                        )
                        .w_full()
                        .max_w(px(980.0))
                        .flex_shrink_0()
                        .children(self.render_library_pages(
                            LibraryPage::Sources,
                            source_count,
                            KNOWLEDGE_PAGE_SIZE,
                            "Sources",
                            cx,
                        )),
                    )
                    .vertical_scrollbar(&self.library_pages.knowledge_scroll),
            )
    }

    fn render_document_card(&self, card: &Card, cx: &Context<'_, Self>) -> AnyElement {
        let id = card.id.clone();
        let keyboard_id = id.clone();
        let id_key = element_key(&id);
        // Web: each tag as `#tag`, so a search by tag shows why it matched.
        let tags = card
            .tags
            .iter()
            .map(|tag| format!("#{tag}"))
            .collect::<Vec<_>>()
            .join(" ");
        div()
            .id(("knowledge-document", id_key))
            .debug_selector(move || format!("desktop-knowledge-document-{id_key}"))
            .rounded(px(8.0))
            .border_1()
            .border_color(theme::longan_800())
            .bg(theme::longan_900().opacity(0.6))
            .px_3()
            .py_3()
            .cursor_pointer()
            .tab_index(0)
            .role(Role::Button)
            .aria_label(format!("Open document {}", card.title))
            .hover(|element| element.border_color(theme::longan_700()))
            .on_click(cx.listener(move |this, _event, window, cx| {
                this.select_knowledge_document(&id, window, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    this.select_knowledge_document(&keyboard_id, window, cx);
                }
            }))
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::rambutan_100())
                    .child(card.title.clone()),
            )
            .child(
                div()
                    .mt_1()
                    .text_size(px(12.0))
                    .text_color(theme::rambutan_500())
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(card.preview.clone()),
            )
            .when(!tags.is_empty(), |card| {
                card.child(
                    div()
                        .mt_1()
                        .text_size(px(12.0))
                        .text_color(theme::blueberry_300().opacity(0.8))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(tags),
                )
            })
            .into_any_element()
    }

    fn render_source_card(&self, card: &Card, cx: &Context<'_, Self>) -> AnyElement {
        let id = card.id.clone();
        let keyboard_id = id.clone();
        let key = element_key(&id);
        div()
            .id(("knowledge-source", key))
            .debug_selector(move || format!("desktop-knowledge-source-{key}"))
            .rounded(px(6.0))
            .px_3()
            .py_2()
            .cursor_pointer()
            .tab_index(0)
            .role(Role::Button)
            .aria_label(format!("Open source {}", card.title))
            .hover(|element| element.bg(theme::longan_900().opacity(0.7)))
            .on_click(cx.listener(move |this, _event, window, cx| {
                this.select_source_clip(&id, window, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    this.select_source_clip(&keyboard_id, window, cx);
                }
            }))
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(theme::rambutan_200())
                    .child(card.title.clone()),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(theme::blueberry_400())
                    .child(card.detail.clone()),
            )
            // Web shows the excerpt's start, which the search also reads.
            .when(!card.preview.trim().is_empty(), |row| {
                row.child(
                    div()
                        .mt_0p5()
                        .text_size(px(12.0))
                        .text_color(theme::rambutan_500())
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(card.preview.clone()),
                )
            })
            .into_any_element()
    }

    pub(super) fn render_document_editor(
        &self,
        document_id: &str,
        compact: bool,
        cx: &Context<'_, Self>,
    ) -> Div {
        let document = self
            .knowledge_documents
            .iter()
            .find(|document| document.id == document_id);
        let Some(document) = document else {
            return self.render_knowledge_page(cx);
        };
        let dirty = self.current_document_dirty(cx);
        let links = document
            .links
            .iter()
            .enumerate()
            .map(|(index, link)| {
                let document_id = document_id.to_string();
                div()
                    .id(("knowledge-link", index))
                    .rounded(px(4.0))
                    .px_2()
                    .py_1()
                    .flex()
                    .justify_between()
                    .cursor_pointer()
                    .hover(|element| element.bg(theme::longan_800().opacity(0.6)))
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.remove_knowledge_link(&document_id, index, cx);
                    }))
                    .child(
                        div()
                            .min_w_0()
                            .text_size(px(12.0))
                            .text_color(theme::rambutan_300())
                            .child(link.label.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(theme::rambutan_500())
                            .child(link.kind.label()),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let citations = document
            .citations
            .iter()
            .filter_map(|citation| {
                let clip = self
                    .source_clips
                    .iter()
                    .find(|clip| clip.id == citation.clip_id)?;
                let document_id = document_id.to_string();
                let clip_id = clip.id.clone();
                Some(
                    div()
                        .id(("knowledge-citation", element_key(&clip_id)))
                        .rounded(px(4.0))
                        .px_2()
                        .py_1()
                        .cursor_pointer()
                        .hover(|element| element.bg(theme::longan_800().opacity(0.6)))
                        .on_click(cx.listener(move |this, _event, _window, cx| {
                            this.remove_citation(&document_id, &clip_id, cx);
                        }))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(theme::rambutan_300())
                                .child(clip.title.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(10.0))
                                .text_color(theme::blueberry_400())
                                .child(source_host(&clip.source_url)),
                        )
                        .into_any_element(),
                )
            })
            .collect::<Vec<_>>();
        let backlinks = backlinks_for(document_id, &self.knowledge_documents)
            .into_iter()
            .map(|backlink| {
                let id = backlink.document_id;
                div()
                    .id(("knowledge-backlink", element_key(&id)))
                    .px_2()
                    .py_1()
                    .text_size(px(12.0))
                    .text_color(theme::blueberry_400())
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _event, window, cx| {
                        this.select_knowledge_document(&id, window, cx);
                    }))
                    .child(backlink.document_title)
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let versions = document
            .versions
            .iter()
            .take(8)
            .map(|version| {
                let document_id = document_id.to_string();
                let version_id = version.id.clone();
                let version_key = version.id.clone();
                div()
                    .id(("knowledge-version", element_key(&version_id)))
                    .debug_selector(move || format!("desktop-knowledge-version-{version_key}"))
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .hover(|element| element.bg(theme::longan_800().opacity(0.6)))
                    .on_click(cx.listener(move |this, _event, window, cx| {
                        this.restore_document_version(&document_id, &version_id, window, cx);
                    }))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme::rambutan_300())
                            .child(version.title.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(theme::rambutan_500())
                            .child(format_epoch_ms(version.recorded_at_ms)),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let attachments = document
            .attachments
            .iter()
            .map(|attachment| {
                let path = knowledge_attachment_path(&attachment.id);
                div()
                    .id(("knowledge-attachment", element_key(&attachment.id)))
                    .rounded(px(4.0))
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .hover(|element| element.bg(theme::longan_800().opacity(0.6)))
                    .on_click(move |_event, _window, cx| {
                        if let Some(path) = path.as_ref() {
                            cx.open_with_system(path);
                        }
                    })
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme::rambutan_300())
                            .child(attachment.file_name.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(theme::rambutan_500())
                            .child(format_bytes(attachment.size_bytes)),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        div()
            .debug_selector(|| "desktop-knowledge-document-editor".to_string())
            .flex_1()
            .h_full()
            .min_w_0()
            .bg(cx.theme().background)
            .flex()
            .flex_col()
            .child(
                div()
                    .px_5()
                    .py_3()
                    .border_b_1()
                    .border_color(theme::longan_800().opacity(0.5))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        web_button(
                            "knowledge-back",
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .aria_label("← Knowledge")
                        .child("← Knowledge")
                        .on_click(cx.listener(
                            |this, _event, window, cx| {
                                this.open_knowledge_view(window, cx);
                            },
                        )),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                web_button(
                                    "ask-agent-about-document",
                                    WebButtonVariant::Ghost,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .debug_selector(|| "desktop-knowledge-ask-agent".to_string())
                                .aria_label("Ask agent")
                                .child("Ask agent")
                                .on_click(cx.listener(
                                    |this, _event, window, cx| {
                                        this.ask_agent_about_current_document(window, cx);
                                    },
                                )),
                            )
                            .child(
                                web_button(
                                    "delete-knowledge-document",
                                    WebButtonVariant::Ghost,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .aria_label("Delete")
                                .child("Delete")
                                .on_click(cx.listener(
                                    |this, _event, window, cx| {
                                        this.request_record_delete(window, cx);
                                    },
                                )),
                            )
                            .child(
                                web_button(
                                    "preview-knowledge-document",
                                    WebButtonVariant::Outline,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .debug_selector(|| "desktop-knowledge-preview".to_string())
                                .aria_label("Preview")
                                .aria_toggled(if self.knowledge.preview {
                                    Toggled::True
                                } else {
                                    Toggled::False
                                })
                                .child(if self.knowledge.preview {
                                    "Edit"
                                } else {
                                    "Preview"
                                })
                                // GPUI clicks a focused button on Enter and
                                // Space too; a key handler would toggle twice.
                                .on_click(cx.listener(
                                    |this, _event, _window, cx| {
                                        this.toggle_knowledge_preview(cx);
                                    },
                                )),
                            )
                            .child(
                                web_button(
                                    "save-knowledge-document",
                                    WebButtonVariant::Primary,
                                    WebButtonSize::Small,
                                    ButtonState::disabled_if(!dirty),
                                )
                                .debug_selector(|| "desktop-knowledge-save".to_string())
                                .aria_label("Save")
                                .child("Save")
                                // Web: disabled until the document changes.
                                .when(dirty, |button| {
                                    button.on_click(cx.listener(|this, _event, _window, cx| {
                                        this.save_current_document(cx);
                                    }))
                                }),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .p_5()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(1120.0))
                            .flex()
                            .when(compact, |content| content.flex_col())
                            .gap_5()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .when(compact, |editor| editor.flex_none().w_full())
                                    .child(
                                        Input::new(&self.knowledge_title_input)
                                            .w_full()
                                            .aria_label("Document title"),
                                    )
                                    .child(if self.knowledge.preview {
                                        self.render_document_preview(cx)
                                    } else {
                                        div()
                                            .debug_selector(|| "desktop-knowledge-body".to_string())
                                            .w_full()
                                            // Web's body field: at least 384px tall.
                                            .h(px(DOCUMENT_BODY_HEIGHT_PX))
                                            .child(
                                                Input::new(&self.knowledge_body_input)
                                                    .aria_label("Document Markdown body")
                                                    .w_full()
                                                    // A multi-line field sizes to the
                                                    // Input, not its box.
                                                    .h_full(),
                                            )
                                            .into_any_element()
                                    })
                                    .child(
                                        div()
                                            .text_size(px(11.0))
                                            .text_color(theme::rambutan_500())
                                            .child("Tags, comma separated"),
                                    )
                                    .child(
                                        Input::new(&self.knowledge_tags_input)
                                            .aria_label("Document tags, comma separated")
                                            .w_full(),
                                    )
                                    .child(
                                        div()
                                            .id("desktop-knowledge-save-line")
                                            .debug_selector(|| {
                                                "desktop-knowledge-save-line".to_string()
                                            })
                                            .role(Role::Status)
                                            .min_h(px(20.0))
                                            .text_size(px(12.0))
                                            .text_color(theme::rambutan_500())
                                            .child(self.document_save_line(cx)),
                                    ),
                            )
                            .child(
                                div()
                                    .w(px(288.0))
                                    .when(compact, |metadata| metadata.w_full())
                                    .flex_shrink_0()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .child(knowledge_meta_panel(
                                        "Connections",
                                        links,
                                        Some(self.render_record_picker_trigger(
                                            RecordPickerTarget::Link,
                                            cx,
                                        )),
                                    ))
                                    .child(knowledge_meta_panel(
                                        "Citations",
                                        citations,
                                        Some(self.render_record_picker_trigger(
                                            RecordPickerTarget::Citation,
                                            cx,
                                        )),
                                    ))
                                    .child(knowledge_meta_panel(
                                        "Attachments",
                                        attachments,
                                        Some(
                                            web_button(
                                                "add-knowledge-attachment",
                                                WebButtonVariant::Ghost,
                                                WebButtonSize::Small,
                                                ButtonState::Enabled,
                                            )
                                            .aria_label("Attach local file")
                                            .child("Attach local file")
                                            .on_click(cx.listener(|this, _event, window, cx| {
                                                this.attach_to_current_document(window, cx);
                                            }))
                                            .into_any_element(),
                                        ),
                                    ))
                                    .child(knowledge_meta_panel(
                                        "Backlinks",
                                        if backlinks.is_empty() {
                                            vec![div()
                                                .px_2()
                                                .text_size(px(13.0))
                                                .text_color(theme::rambutan_500())
                                                .child(document_copy::NO_BACKLINKS)
                                                .into_any_element()]
                                        } else {
                                            backlinks
                                        },
                                        None,
                                    ))
                                    .child(knowledge_meta_panel("History", versions, None)),
                            ),
                    ),
            )
    }

    /// Web's preview `article`: the editor's current Markdown, rendered by the
    /// chat replies' renderer. Like the web's untrusted renderer it drops raw
    /// HTML and keeps images as links, so a preview never loads a remote
    /// image.
    fn render_document_preview(&self, cx: &Context<'_, Self>) -> AnyElement {
        let markdown = preview_markdown(&self.knowledge_body_input.read(cx).value());
        div()
            .id("knowledge-document-preview")
            .debug_selector(|| "desktop-knowledge-document-preview".to_string())
            .role(Role::Article)
            .aria_label("Document preview")
            .w_full()
            .min_h(px(320.0))
            .rounded(px(8.0))
            .bg(theme::longan_900().opacity(0.3))
            .border_1()
            .border_color(theme::longan_800())
            .p_4()
            .text_size(px(14.0))
            .text_color(theme::rambutan_200())
            .child(
                TextView::markdown("knowledge-document-preview-markdown", markdown)
                    .style(desktop_markdown_style(cx))
                    .selectable(true),
            )
            .into_any_element()
    }

    pub(super) fn render_source_clip_editor(&self, creating: bool, cx: &Context<'_, Self>) -> Div {
        let source_url = match &self.active_view {
            AppView::ClipDetail { clip_id } => self
                .source_clips
                .iter()
                .find(|clip| clip.id == *clip_id)
                .map(|clip| clip.source_url.clone()),
            _ => None,
        };
        div()
            .debug_selector(|| "desktop-knowledge-source-editor".to_string())
            .flex_1()
            .h_full()
            .min_w_0()
            .bg(cx.theme().background)
            .flex()
            .flex_col()
            .child(
                div()
                    .px_5()
                    .py_3()
                    .border_b_1()
                    .border_color(theme::longan_800().opacity(0.5))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        web_button(
                            "source-back",
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .aria_label("← Knowledge")
                        .child("← Knowledge")
                        .on_click(cx.listener(
                            |this, _event, window, cx| {
                                this.open_knowledge_view(window, cx);
                            },
                        )),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .when_some(source_url, |row, url| {
                                row.child(
                                    web_button(
                                        "ask-agent-about-source",
                                        WebButtonVariant::Ghost,
                                        WebButtonSize::Small,
                                        ButtonState::Enabled,
                                    )
                                    .aria_label("Ask agent")
                                    .child("Ask agent")
                                    .on_click(cx.listener(
                                        |this, _event, window, cx| {
                                            this.ask_agent_about_current_source(window, cx);
                                        },
                                    )),
                                )
                                .child(
                                    web_button(
                                        "open-source-original",
                                        WebButtonVariant::Ghost,
                                        WebButtonSize::Small,
                                        ButtonState::Enabled,
                                    )
                                    .aria_label("Open original")
                                    .child("Open original")
                                    .on_click(move |_event, _window, cx| cx.open_url(&url)),
                                )
                                .child(
                                    web_button(
                                        "delete-source-clip",
                                        WebButtonVariant::Ghost,
                                        WebButtonSize::Small,
                                        ButtonState::Enabled,
                                    )
                                    .aria_label("Delete")
                                    .child("Delete")
                                    .on_click(cx.listener(
                                        |this, _event, window, cx| {
                                            this.request_record_delete(window, cx);
                                        },
                                    )),
                                )
                            })
                            .child(
                                web_button(
                                    "save-source-clip",
                                    WebButtonVariant::Primary,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .debug_selector(|| "desktop-knowledge-source-save".to_string())
                                .aria_label(if creating { "Save clip" } else { "Save source" })
                                .child(if creating { "Save clip" } else { "Save source" })
                                .on_click(cx.listener(
                                    |this, _event, _window, cx| {
                                        this.save_source_clip(cx);
                                    },
                                )),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .p_5()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(720.0))
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(theme::longan_800())
                            .bg(theme::longan_900().opacity(0.4))
                            .p_4()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(theme::rambutan_500())
                                    .child("Title"),
                            )
                            .child(
                                Input::new(&self.clip_title_input)
                                    .aria_label("Source title, required")
                                    .w_full(),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(theme::rambutan_500())
                                    .child("Source URL"),
                            )
                            .child(
                                Input::new(&self.clip_url_input)
                                    .aria_label("Source URL, required")
                                    .w_full(),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_3()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .child(
                                                div()
                                                    .text_size(px(11.0))
                                                    .text_color(theme::rambutan_500())
                                                    .child("Publication"),
                                            )
                                            .child(
                                                Input::new(&self.clip_source_name_input)
                                                    .aria_label("Source name")
                                                    .w_full(),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .child(
                                                div()
                                                    .text_size(px(11.0))
                                                    .text_color(theme::rambutan_500())
                                                    .child("Author"),
                                            )
                                            .child(
                                                Input::new(&self.clip_author_input)
                                                    .aria_label("Author")
                                                    .w_full(),
                                            ),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_3()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .child(
                                                div()
                                                    .text_size(px(11.0))
                                                    .text_color(theme::rambutan_500())
                                                    .child("Published"),
                                            )
                                            .child(
                                                Input::new(&self.clip_published_input)
                                                    .aria_label("Publication date")
                                                    .w_full(),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .child(
                                                div()
                                                    .text_size(px(11.0))
                                                    .text_color(theme::rambutan_500())
                                                    .child("Tags"),
                                            )
                                            .child(
                                                Input::new(&self.clip_tags_input)
                                                    .aria_label("Source tags, comma separated")
                                                    .w_full(),
                                            ),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(theme::rambutan_500())
                                    .child("Excerpt or note"),
                            )
                            .child(
                                Input::new(&self.clip_excerpt_input)
                                    .aria_label("Source excerpt")
                                    .w_full()
                                    // Web's 12-row excerpt field.
                                    .h(px(SOURCE_EXCERPT_HEIGHT_PX)),
                            ),
                    ),
            )
    }
}

/// A library section: its title and how many records match the search
/// (web's count beside each heading), then its cards.
fn knowledge_section(
    title: &'static str,
    count: usize,
    rows: Vec<AnyElement>,
    empty: &'static str,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .items_baseline()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .text_size(px(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::rambutan_100())
                        .child(title),
                )
                .child(
                    div()
                        .debug_selector(move || format!("desktop-knowledge-count-{title}"))
                        .font_features(theme::tabular_nums())
                        .text_size(px(12.0))
                        .text_color(theme::rambutan_500())
                        .child(count.to_string()),
                ),
        )
        .when(rows.is_empty(), |element| {
            element.child(
                div()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(theme::longan_800())
                    .px_3()
                    .py_3()
                    .text_size(px(12.0))
                    .text_color(theme::rambutan_500())
                    .child(empty),
            )
        })
        .children(rows)
}

fn knowledge_meta_panel(
    title: &'static str,
    rows: Vec<AnyElement>,
    action: Option<AnyElement>,
) -> Div {
    div()
        .rounded(px(8.0))
        .border_1()
        .border_color(theme::longan_800())
        .bg(theme::longan_900().opacity(0.4))
        .p_3()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                // Web `text-label-sm uppercase`.
                .text_size(px(12.0))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::rambutan_500())
                .child(title.to_uppercase()),
        )
        // Web puts each panel's control above its list.
        .children(action)
        .children(rows)
}

/// A document body as the preview shows it: without raw HTML, and with every
/// image written as a link to its source (web `render_untrusted`).
fn preview_markdown(body: &str) -> String {
    markdown_with_allowed_images(body, &BTreeSet::new())
}

fn format_epoch_ms(timestamp_ms: i64) -> String {
    Local
        .timestamp_millis_opt(timestamp_ms)
        .single()
        .map_or_else(
            || "Earlier".to_string(),
            |date| date.format("%b %-d, %-I:%M%P").to_string(),
        )
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes.to_f64_lossy() / 1_048_576.0)
    } else if bytes >= 1_024 {
        format!("{:.1} KB", bytes.to_f64_lossy() / 1_024.0)
    } else {
        format!("{bytes} B")
    }
}

fn mime_guess_from_name(file_name: &str) -> &'static str {
    match std::path::Path::new(file_name)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "pdf" => "application/pdf",
        "md" => "text/markdown",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    }
}

/// The document editor and web-clip form fields.
pub(super) struct KnowledgeInputs {
    pub(super) knowledge_title_input: Entity<InputState>,
    pub(super) knowledge_body_input: Entity<InputState>,
    pub(super) knowledge_tags_input: Entity<InputState>,
    pub(super) clip_title_input: Entity<InputState>,
    pub(super) clip_url_input: Entity<InputState>,
    pub(super) clip_source_name_input: Entity<InputState>,
    pub(super) clip_author_input: Entity<InputState>,
    pub(super) clip_published_input: Entity<InputState>,
    pub(super) clip_tags_input: Entity<InputState>,
    pub(super) clip_excerpt_input: Entity<InputState>,
}

impl Workspace {
    /// The document editor and web-clip form fields; edits redraw.
    pub(super) fn new_knowledge_inputs(
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> KnowledgeInputs {
        let knowledge_title_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Document title"));
        subscriptions.push(redraw_on_edit(&knowledge_title_input, window, cx));
        let knowledge_body_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Write Markdown…")
                .multi_line(true)
                .rows(18)
        });
        subscriptions.push(redraw_on_edit(&knowledge_body_input, window, cx));
        let knowledge_tags_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("research, product, decision"));
        subscriptions.push(redraw_on_edit(&knowledge_tags_input, window, cx));
        let clip_title_input = cx.new(|cx| InputState::new(window, cx).placeholder("Source title"));
        subscriptions.push(redraw_on_edit(&clip_title_input, window, cx));
        let clip_url_input = cx.new(|cx| InputState::new(window, cx).placeholder("https://…"));
        subscriptions.push(redraw_on_edit(&clip_url_input, window, cx));
        let clip_source_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Publication"));
        subscriptions.push(redraw_on_edit(&clip_source_name_input, window, cx));
        let clip_author_input = cx.new(|cx| InputState::new(window, cx).placeholder("Author"));
        subscriptions.push(redraw_on_edit(&clip_author_input, window, cx));
        let clip_published_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("YYYY-MM-DD"));
        subscriptions.push(redraw_on_edit(&clip_published_input, window, cx));
        let clip_tags_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("research, product"));
        subscriptions.push(redraw_on_edit(&clip_tags_input, window, cx));
        let clip_excerpt_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Excerpt or note")
                .multi_line(true)
                .rows(10)
        });
        subscriptions.push(redraw_on_edit(&clip_excerpt_input, window, cx));
        KnowledgeInputs {
            knowledge_title_input,
            knowledge_body_input,
            knowledge_tags_input,
            clip_title_input,
            clip_url_input,
            clip_source_name_input,
            clip_author_input,
            clip_published_input,
            clip_tags_input,
            clip_excerpt_input,
        }
    }
}

#[cfg(test)]
mod parity_tests;

#[cfg(test)]
mod tests {
    use super::{mime_guess_from_name, preview_markdown};

    #[test]
    fn previews_drop_raw_html_and_keep_images_as_links() {
        let body = "# Plan\n\n<script>alert(1)</script>\n\n![Chart](https://example.invalid/c.png) and **bold**";
        let markdown = preview_markdown(body);
        assert!(!markdown.contains("<script>"), "{markdown}");
        assert!(markdown.contains("[Chart](https://example.invalid/c.png)"));
        assert!(
            !markdown.contains("![Chart]"),
            "a preview never loads the image"
        );
        assert!(markdown.contains("# Plan") && markdown.contains("**bold**"));
    }

    #[test]
    fn mime_guess_covers_common_knowledge_files() {
        assert_eq!(mime_guess_from_name("brief.PDF"), "application/pdf");
        assert_eq!(mime_guess_from_name("notes.md"), "text/markdown");
    }
}
