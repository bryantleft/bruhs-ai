//! Native Knowledge surface matching the web library's workflows and Bruhs
//! hierarchy. Document and source mutations share the desktop's one atomic
//! state snapshot; attachment payloads are copied beside that snapshot.

use bruhs_models::num::ToFloatLossy;
use gpui::prelude::*;
use gpui::*;
use gpui_component::input::Input;

use super::knowledge_storage::{self, attachment_path as knowledge_attachment_path};
use super::*;

impl Workspace {
    pub(super) fn open_knowledge_view(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) {
        self.active_view = AppView::Knowledge;
        self.clip_create_visible = false;
        self.compact_sidebar_open = false;
        self.close_transient_surfaces();
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
        self.clip_create_visible = false;
        self.persist_and_notify(cx);
        self.knowledge_title_input.update(cx, |input, cx| {
            input.focus(window, cx);
        });
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
                self.clip_create_visible = false;
            }
        }
        self.persistence_error = None;
        self.persist_and_notify(cx);
    }

    fn delete_current_document(&mut self, cx: &mut Context<'_, Self>) {
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

    fn delete_current_source_clip(&mut self, cx: &mut Context<'_, Self>) {
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

    fn add_next_knowledge_link(&mut self, cx: &mut Context<'_, Self>) {
        let AppView::DocumentDetail { document_id } = &self.active_view else {
            return;
        };
        let existing = self
            .knowledge_documents
            .iter()
            .find(|document| document.id == *document_id)
            .map(|document| document.links.clone())
            .unwrap_or_default();
        let candidates = self.knowledge_link_candidates(document_id, cx);
        let Some(link) = candidates.into_iter().find(|candidate| {
            !existing
                .iter()
                .any(|item| item.kind == candidate.kind && item.target_id == candidate.target_id)
        }) else {
            return;
        };
        if let Some(document) = self
            .knowledge_documents
            .iter_mut()
            .find(|document| document.id == *document_id)
        {
            let mut links = document.links.clone();
            links.push(link);
            document.update_links(links, EpochMs::now().as_millis());
            self.persist_and_notify(cx);
        }
    }

    fn knowledge_link_candidates(&self, current_id: &str, cx: &App) -> Vec<KnowledgeLink> {
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

    fn add_next_citation(&mut self, cx: &mut Context<'_, Self>) {
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
        let Some(clip) = self.source_clips.iter().find(|clip| {
            !document
                .citations
                .iter()
                .any(|citation| citation.clip_id == clip.id)
        }) else {
            return;
        };
        let mut citations = document.citations.clone();
        citations.push(KnowledgeCitation {
            clip_id: clip.id.clone(),
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
        let documents = self
            .knowledge_documents
            .iter()
            .map(|document| self.render_document_card(document, cx))
            .collect::<Vec<_>>();
        let clips = self
            .source_clips
            .iter()
            .map(|clip| self.render_source_card(clip, cx))
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
                                    .child("Living local context, links, and cited sources"),
                            ),
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
                                .child("Ask agent")
                                .on_click(cx.listener(
                                    |this, _event, window, cx| {
                                        this.ask_agent_about_current_document(window, cx);
                                    },
                                )),
                            )
                            .child(
                                web_button(
                                    "new-source-clip",
                                    WebButtonVariant::Ghost,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .debug_selector(|| "desktop-knowledge-new-clip".to_string())
                                .child("+ Web clip")
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
                                .child("+ Document")
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
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .p_5()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(980.0))
                            .flex()
                            .flex_col()
                            .gap_5()
                            .child(knowledge_section(
                                "Documents",
                                documents,
                                "No documents yet.",
                            ))
                            .child(knowledge_section(
                                "Sources",
                                clips,
                                "Save a web source to cite it later.",
                            )),
                    ),
            )
    }

    fn render_document_card(
        &self,
        document: &KnowledgeDocument,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let id = document.id.clone();
        let keyboard_id = id.clone();
        let id_key = element_key(&id);
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
            .aria_label(format!("Open document {}", document.title))
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
                    .child(document.title.clone()),
            )
            .child(
                div()
                    .mt_1()
                    .text_size(px(12.0))
                    .text_color(theme::rambutan_500())
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(document_preview(&document.body_markdown)),
            )
            .into_any_element()
    }

    fn render_source_card(&self, clip: &SourceClip, cx: &Context<'_, Self>) -> AnyElement {
        let id = clip.id.clone();
        let keyboard_id = id.clone();
        div()
            .id(("knowledge-source", element_key(&id)))
            .rounded(px(6.0))
            .px_3()
            .py_2()
            .cursor_pointer()
            .tab_index(0)
            .role(Role::Button)
            .aria_label(format!("Open source {}", clip.title))
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
                    .child(clip.title.clone()),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(theme::blueberry_400())
                    .child(source_host(&clip.source_url)),
            )
            .into_any_element()
    }

    pub(super) fn render_document_editor(&self, document_id: &str, cx: &Context<'_, Self>) -> Div {
        let document = self
            .knowledge_documents
            .iter()
            .find(|document| document.id == document_id);
        let Some(document) = document else {
            return self.render_knowledge_page(cx);
        };
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
                                    "delete-knowledge-document",
                                    WebButtonVariant::Ghost,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .child("Delete")
                                .on_click(cx.listener(
                                    |this, _event, _window, cx| {
                                        this.delete_current_document(cx);
                                    },
                                )),
                            )
                            .child(
                                web_button(
                                    "save-knowledge-document",
                                    WebButtonVariant::Primary,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .debug_selector(|| "desktop-knowledge-save".to_string())
                                .child("Save")
                                .on_click(cx.listener(
                                    |this, _event, _window, cx| {
                                        this.save_current_document(cx);
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
                            .max_w(px(1120.0))
                            .flex()
                            .gap_5()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .child(Input::new(&self.knowledge_title_input).w_full())
                                    .child(Input::new(&self.knowledge_body_input).w_full())
                                    .child(
                                        div()
                                            .text_size(px(11.0))
                                            .text_color(theme::rambutan_500())
                                            .child("Tags — comma separated"),
                                    )
                                    .child(Input::new(&self.knowledge_tags_input).w_full()),
                            )
                            .child(
                                div()
                                    .w(px(288.0))
                                    .flex_shrink_0()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .child(knowledge_meta_panel(
                                        "Connections",
                                        links,
                                        Some(
                                            web_button(
                                                "add-knowledge-link",
                                                WebButtonVariant::Ghost,
                                                WebButtonSize::Small,
                                                ButtonState::Enabled,
                                            )
                                            .child("+ Connect next record")
                                            .on_click(cx.listener(|this, _event, _window, cx| {
                                                this.add_next_knowledge_link(cx);
                                            }))
                                            .into_any_element(),
                                        ),
                                    ))
                                    .child(knowledge_meta_panel(
                                        "Citations",
                                        citations,
                                        Some(
                                            web_button(
                                                "add-knowledge-citation",
                                                WebButtonVariant::Ghost,
                                                WebButtonSize::Small,
                                                ButtonState::disabled_if(
                                                    self.source_clips.is_empty(),
                                                ),
                                            )
                                            .child("+ Cite next source")
                                            .on_click(cx.listener(|this, _event, _window, cx| {
                                                this.add_next_citation(cx);
                                            }))
                                            .into_any_element(),
                                        ),
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
                                            .child("Attach local file")
                                            .on_click(cx.listener(|this, _event, window, cx| {
                                                this.attach_to_current_document(window, cx);
                                            }))
                                            .into_any_element(),
                                        ),
                                    ))
                                    .child(knowledge_meta_panel("Backlinks", backlinks, None))
                                    .child(knowledge_meta_panel("History", versions, None)),
                            ),
                    ),
            )
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
                                    .child("Delete")
                                    .on_click(cx.listener(
                                        |this, _event, _window, cx| {
                                            this.delete_current_source_clip(cx);
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
                            .child(Input::new(&self.clip_title_input).w_full())
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(theme::rambutan_500())
                                    .child("Source URL"),
                            )
                            .child(Input::new(&self.clip_url_input).w_full())
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
                                                Input::new(&self.clip_source_name_input).w_full(),
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
                                            .child(Input::new(&self.clip_author_input).w_full()),
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
                                            .child(Input::new(&self.clip_published_input).w_full()),
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
                                            .child(Input::new(&self.clip_tags_input).w_full()),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(theme::rambutan_500())
                                    .child("Excerpt or note"),
                            )
                            .child(Input::new(&self.clip_excerpt_input).w_full()),
                    ),
            )
    }
}

fn knowledge_section(title: &'static str, rows: Vec<AnyElement>, empty: &'static str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_size(px(13.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::rambutan_100())
                .child(title),
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
                .text_size(px(10.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::rambutan_500())
                .child(title.to_uppercase()),
        )
        .children(rows)
        .children(action)
}

fn document_preview(markdown: &str) -> String {
    markdown
        .lines()
        .find(|line| !line.trim().is_empty())
        .map_or_else(
            || "Empty document".to_string(),
            |line| {
                line.trim_start_matches('#')
                    .trim()
                    .chars()
                    .take(100)
                    .collect()
            },
        )
}

fn source_host(url: &str) -> String {
    url.trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or(url)
        .to_string()
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
mod tests {
    use super::{document_preview, mime_guess_from_name, source_host};

    #[test]
    fn document_preview_and_source_host_match_web_rules() {
        assert_eq!(document_preview("\n## Decision\nBody"), "Decision");
        assert_eq!(source_host("https://example.com/research"), "example.com");
    }

    #[test]
    fn mime_guess_covers_common_knowledge_files() {
        assert_eq!(mime_guess_from_name("brief.PDF"), "application/pdf");
        assert_eq!(mime_guess_from_name("notes.md"), "text/markdown");
    }
}
