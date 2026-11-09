//! Local-first knowledge library: lightweight Markdown documents, source
//! clips, typed links, citations, backlinks, attachments, and version history.

use bruhs_models::num::{ToFloatLossy, ToIntSaturating};
use bruhs_models::{
    backlinks_for, KnowledgeAttachment, KnowledgeCitation, KnowledgeDocument, KnowledgeLink,
    KnowledgeLinkKind, SourceClip,
};
use leptos::prelude::*;
use wasm_bindgen::{JsCast, JsValue};

use crate::state::{
    AppView, ComposerDraft, KnowledgeCreateClipRequest, KnowledgeDocumentsSignal,
    KnowledgeSaveRequest, MeetingsSignal, PeopleSignal, ProjectsSignal, SetActiveView,
    SetKnowledgeDocumentsSignal, SetSourceClipsSignal, SourceClipsSignal, WorkItemsSignal,
};
use crate::storage;
use crate::ui::{Dialog, DropdownOption, Icon, Input, Textarea};

const MAX_ATTACHMENT_BYTES: u64 = 10 * 1024 * 1024;
mod library;
mod record_picker;
use record_picker::RecordPicker;

#[component]
pub fn KnowledgePage(
    #[prop(optional)] document_id: Option<String>,
    #[prop(optional)] clip_id: Option<String>,
) -> impl IntoView {
    let executor = expect_context::<crate::workflow_executor::WorkflowExecutor>();
    let content = match (document_id, clip_id) {
        (Some(document_id), _) => view! { <DocumentEditor document_id /> }.into_any(),
        (_, Some(clip_id)) => view! { <ClipEditor clip_id /> }.into_any(),
        _ => view! { <KnowledgeLibrary /> }.into_any(),
    };
    view! {
        <div class="flex h-full min-h-0 flex-col">
            <Show when=move || executor.load_error.get().is_some()>
                <section aria-label="Knowledge loading status" class="shrink-0 border-b border-longan-800 px-4 py-3 text-body-sm text-rambutan-300">
                    <p role="alert">"Local records could not be loaded. Existing storage was left unchanged; document and source changes made here are kept."</p>
                    <button type="button" class="mt-2 min-h-11 rounded-grape bg-longan-800 px-3 py-2 text-label text-rambutan-200 hover:bg-longan-700 disabled:opacity-40 cursor-pointer"
                        disabled=move || executor.loading.get() on:click=move |_| executor.load()>"Retry loading records"</button>
                </section>
            </Show>
            <div class="min-h-0 flex-1">{content}</div>
        </div>
    }
}

#[component]
fn KnowledgeLibrary() -> impl IntoView {
    let executor = expect_context::<crate::workflow_executor::WorkflowExecutor>();
    let KnowledgeDocumentsSignal(documents) = expect_context::<KnowledgeDocumentsSignal>();
    let SetKnowledgeDocumentsSignal(set_documents) =
        expect_context::<SetKnowledgeDocumentsSignal>();
    let SourceClipsSignal(clips) = expect_context::<SourceClipsSignal>();
    let SetSourceClipsSignal(set_clips) = expect_context::<SetSourceClipsSignal>();
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    let KnowledgeCreateClipRequest(create_clip_request) =
        expect_context::<KnowledgeCreateClipRequest>();

    let query = RwSignal::new(String::new());
    let clip_dialog_open = RwSignal::new(false);
    let clip_title = RwSignal::new(String::new());
    let clip_url = RwSignal::new(String::new());
    let clip_excerpt = RwSignal::new(String::new());
    let clip_error = RwSignal::new(Option::<String>::None);
    Effect::new(move |_| {
        if create_clip_request.take() {
            clip_dialog_open.set(true);
        }
    });

    let document_page = RwSignal::new(0usize);
    let source_page = RwSignal::new(0usize);
    // Data changes rebuild the lightweight index, not every keystroke. Neither
    // index nor page cards copy document history, links or attachment metadata.
    let document_index =
        Memo::new(move |_| documents.with(|items| library::Index::documents(items)));
    let source_index = Memo::new(move |_| clips.with(|items| library::Index::sources(items)));
    let matching_documents =
        Memo::new(move |_| document_index.with(|index| index.matching(&query.get())));
    let matching_sources =
        Memo::new(move |_| source_index.with(|index| index.matching(&query.get())));
    let visible_documents = Memo::new(move |_| {
        document_index.with(|index| {
            matching_documents.with(|matches| index.page(matches, document_page.get()))
        })
    });
    let visible_clips = Memo::new(move |_| {
        source_index
            .with(|index| matching_sources.with(|matches| index.page(matches, source_page.get())))
    });

    let new_document = move |_| {
        let document = KnowledgeDocument::new("Untitled", "", crate::clock::now_ms());
        let id = document.id.clone();
        set_documents.update(|items| items.insert(0, document.clone()));
        storage::persist_knowledge_document(None, document);
        set_active_view.set(AppView::DocumentDetail { document_id: id });
    };
    let save_clip = move || {
        let title = clip_title.get_untracked();
        let url = clip_url.get_untracked();
        if title.trim().is_empty() || url.trim().is_empty() {
            clip_error.set(Some("A title and source URL are required.".to_string()));
            return;
        }
        let clip = SourceClip::new(
            title,
            url,
            clip_excerpt.get_untracked(),
            crate::clock::now_ms(),
        );
        let id = clip.id.clone();
        set_clips.update(|items| items.insert(0, clip.clone()));
        storage::persist_source_clip(None, clip);
        clip_dialog_open.set(false);
        clip_title.set(String::new());
        clip_url.set(String::new());
        clip_excerpt.set(String::new());
        clip_error.set(None);
        set_active_view.set(AppView::ClipDetail { clip_id: id });
    };

    view! {
        <div class="h-full overflow-auto bg-longan-950 text-rambutan-300">
            <div class="mx-auto w-full max-w-5xl space-y-6 px-4 py-5 sm:px-6">
                <header class="flex flex-wrap items-start justify-between gap-3">
                    <div>
                        <p class="text-label-sm uppercase tracking-[0.16em] text-persimmon-300">"Knowledge"</p>
                        <h1 class="mt-1 text-title text-rambutan-100">"Living context, close at hand"</h1>
                        <p class="mt-1 max-w-2xl text-body-sm text-rambutan-500">
                            "Write in Markdown, connect work, and keep every source traceable. Everything stays local and searchable."
                        </p>
                    </div>
                    <div class="flex items-center gap-2">
                        <button type="button" class="rounded-grape px-3 py-2 text-label text-rambutan-300 inset-ring inset-ring-longan-700 hover:bg-longan-800/60 cursor-pointer" on:click=move |_| clip_dialog_open.set(true)>
                            "+ Web clip"
                        </button>
                        <button type="button" class="rounded-grape bg-persimmon-500 px-3 py-2 text-label text-longan-950 hover:bg-persimmon-400 cursor-pointer" on:click=new_document>
                            "+ Document"
                        </button>
                    </div>
                </header>

                <Input
                    aria_label="Search knowledge"
                    input_type="search".to_string()
                    placeholder="Search documents, tags, clips, and sources…".to_string()
                    value=Signal::derive(move || query.get())
                    on_input=Callback::new(move |value| {
                        query.set(value);
                        document_page.set(0);
                        source_page.set(0);
                    })
                />

                <section aria-labelledby="documents-heading" class="space-y-2">
                    <div class="flex items-baseline justify-between gap-3">
                        <h2 id="documents-heading" class="text-label text-rambutan-200">"Documents"</h2>
                        <span class="text-label-sm tabular-nums text-rambutan-500">{move || matching_documents.with(Vec::len)}</span>
                    </div>
                    {move || {
                        let items = visible_documents.get();
                        if items.is_empty() {
                            let text = if executor.ready.get() { "No documents match yet." } else if executor.load_error.get().is_some() { "Documents could not be loaded." } else { "Loading documents…" };
                            view! { <EmptyKnowledge icon="file-text" text /> }.into_any()
                        } else {
                            view! {
                                <div class="grid gap-2 sm:grid-cols-2">
                                    {items.into_iter().map(|document| {
                                        let id = document.id.clone();
                                        let tags = document.tags.iter().map(|tag| format!("#{tag}")).collect::<Vec<_>>().join(" ");
                                        let has_tags = !tags.is_empty();
                                        view! {
                                            <button type="button" data-knowledge-document-id=document.id class="rounded-grape bg-longan-900/60 p-3 text-left inset-ring inset-ring-longan-800 hover:bg-longan-900 hover:inset-ring-longan-700 cursor-pointer" on:click=move |_| set_active_view.set(AppView::DocumentDetail { document_id: id.clone() })>
                                                <span class="flex items-start gap-3">
                                                    <span class="flex size-8 shrink-0 items-center justify-center rounded-seed bg-blueberry-500/10 text-blueberry-300"><Icon name="file-text" size=16 /></span>
                                                    <span class="min-w-0 flex-1">
                                                        <span class="block truncate text-label text-rambutan-100">{document.title}</span>
                                                        <span class="mt-0.5 block truncate text-body-sm text-rambutan-500">{document.preview}</span>
                                                        <Show when=move || has_tags><span class="mt-1 block truncate text-label-sm text-blueberry-300/80">{tags.clone()}</span></Show>
                                                    </span>
                                                </span>
                                            </button>
                                        }
                                    }).collect_view()}
                                </div>
                            }.into_any()
                        }
                    }}
                    <LibraryPagination label="Document pages" noun="documents" page=document_page count=Signal::derive(move || matching_documents.with(Vec::len)) />
                </section>

                <section aria-labelledby="sources-heading" class="space-y-2">
                    <div class="flex items-baseline justify-between gap-3">
                        <h2 id="sources-heading" class="text-label text-rambutan-200">"Sources"</h2>
                        <span class="text-label-sm tabular-nums text-rambutan-500">{move || matching_sources.with(Vec::len)}</span>
                    </div>
                    {move || {
                        let items = visible_clips.get();
                        if items.is_empty() {
                            let text = if executor.ready.get() { "Save a web source to cite it later." } else if executor.load_error.get().is_some() { "Sources could not be loaded." } else { "Loading sources…" };
                            view! { <EmptyKnowledge icon="bookmark" text /> }.into_any()
                        } else {
                            view! {
                                <div class="space-y-1">
                                    {items.into_iter().map(|clip| {
                                        let id = clip.id.clone();
                                        view! {
                                            <button type="button" data-source-clip-id=clip.id class="flex w-full items-start gap-3 rounded-grape px-3 py-2.5 text-left hover:bg-longan-900/70 cursor-pointer" on:click=move |_| set_active_view.set(AppView::ClipDetail { clip_id: id.clone() })>
                                                <span class="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-seed bg-persimmon-500/10 text-persimmon-300"><Icon name="bookmark" size=14 /></span>
                                                <span class="min-w-0 flex-1">
                                                    <span class="block truncate text-label text-rambutan-100">{clip.title}</span>
                                                    <span class="block truncate text-label-sm text-blueberry-300/80">{clip.detail}</span>
                                                    <span class="mt-0.5 block truncate text-body-sm text-rambutan-500">{clip.preview}</span>
                                                </span>
                                            </button>
                                        }
                                    }).collect_view()}
                                </div>
                            }.into_any()
                        }
                    }}
                    <LibraryPagination label="Source pages" noun="sources" page=source_page count=Signal::derive(move || matching_sources.with(Vec::len)) />
                </section>
            </div>
        </div>

        <Show when=move || clip_dialog_open.get()>
            <Dialog open=Signal::derive(move || clip_dialog_open.get()) label="Save a web clip" on_close=Callback::new(move |_| clip_dialog_open.set(false))>
                <div class="border-b border-longan-700/70 px-4 py-3">
                    <h2 class="text-h4 text-rambutan-100">"Save a web clip"</h2>
                    <p class="mt-1 text-body-sm text-rambutan-500">
                        "Keep the source URL with the excerpt so future citations stay verifiable."
                    </p>
                </div>
                <form class="space-y-3 px-4 py-3" on:submit=move |event| { event.prevent_default(); save_clip(); }>
                    <div class="space-y-1">
                        <label class="text-label-sm text-rambutan-400">"Title"</label>
                        <Input
                            aria_label="Clip title"
                            value=Signal::derive(move || clip_title.get())
                            on_input=Callback::new(move |value| clip_title.set(value))
                            autofocus=true
                        />
                    </div>
                    <div class="space-y-1">
                        <label class="text-label-sm text-rambutan-400">"Source URL"</label>
                        <Input
                            aria_label="Source URL"
                            input_type="url".to_string()
                            value=Signal::derive(move || clip_url.get())
                            on_input=Callback::new(move |value| clip_url.set(value))
                            placeholder="https://…".to_string()
                        />
                    </div>
                    <div class="space-y-1">
                        <label class="text-label-sm text-rambutan-400">"Excerpt or note"</label>
                        <Textarea
                            aria_label="Excerpt or note"
                            rows=6
                            value=Signal::derive(move || clip_excerpt.get())
                            on_input=Callback::new(move |value| clip_excerpt.set(value))
                        />
                    </div>
                    <Show when=move || clip_error.get().is_some()>
                        <p role="alert" class="text-body-sm text-guava-300">{move || clip_error.get().unwrap_or_default()}</p>
                    </Show>
                    <div class="flex justify-end gap-2">
                        <button
                            type="button"
                            class="rounded-grape px-3 py-2 text-label text-rambutan-400 hover:bg-longan-800 cursor-pointer"
                            on:click=move |_| clip_dialog_open.set(false)
                        >
                            "Cancel"
                        </button>
                        <button type="submit" class="rounded-grape bg-persimmon-500 px-3 py-2 text-label text-longan-950 hover:bg-persimmon-400 cursor-pointer">
                            "Save clip"
                        </button>
                    </div>
                </form>
            </Dialog>
        </Show>
    }
}

#[component]
fn LibraryPagination(
    label: &'static str,
    noun: &'static str,
    page: RwSignal<usize>,
    count: Signal<usize>,
) -> impl IntoView {
    let current = move || page.get().min(library::last_page(count.get()));
    view! {
        <Show when=move || { count.get() > library::PAGE_SIZE }>
            <nav aria-label=label class="flex items-center justify-between gap-3 pt-1 text-label-sm text-rambutan-400">
                <button type="button" aria-label=format!("Previous {noun}") disabled=move || current() == 0
                    class="min-h-11 rounded-grape px-3 py-2 inset-ring inset-ring-longan-700 hover:bg-longan-900 disabled:opacity-40 cursor-pointer"
                    on:click=move |_| page.set(current().saturating_sub(1))>"Previous"</button>
                <span class="tabular-nums" aria-live="polite">{move || format!("Page {} of {}", current() + 1, library::last_page(count.get()) + 1)}</span>
                <button type="button" aria-label=format!("Next {noun}") disabled=move || { current() >= library::last_page(count.get()) }
                    class="min-h-11 rounded-grape px-3 py-2 inset-ring inset-ring-longan-700 hover:bg-longan-900 disabled:opacity-40 cursor-pointer"
                    on:click=move |_| page.set(current().saturating_add(1))>"Next"</button>
            </nav>
        </Show>
    }
}

#[component]
fn EmptyKnowledge(icon: &'static str, text: &'static str) -> impl IntoView {
    view! {
        <div class="flex items-center gap-3 rounded-grape bg-longan-900/30 px-3 py-3 text-body-sm text-rambutan-500 inset-ring inset-ring-longan-800">
            <Icon name=icon size=16 /> <span>{text}</span>
        </div>
    }
}

#[component]
fn DocumentEditor(document_id: String) -> impl IntoView {
    let executor = expect_context::<crate::workflow_executor::WorkflowExecutor>();
    let saves = expect_context::<crate::task_saves::TaskSaves>();
    let KnowledgeDocumentsSignal(documents) = expect_context::<KnowledgeDocumentsSignal>();
    let SetKnowledgeDocumentsSignal(set_documents) =
        expect_context::<SetKnowledgeDocumentsSignal>();
    let SourceClipsSignal(clips) = expect_context::<SourceClipsSignal>();
    let WorkItemsSignal(work_items) = expect_context::<WorkItemsSignal>();
    let ProjectsSignal(projects) = expect_context::<ProjectsSignal>();
    let MeetingsSignal(meetings) = expect_context::<MeetingsSignal>();
    let PeopleSignal(people) = expect_context::<PeopleSignal>();
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    let KnowledgeSaveRequest(save_request) = expect_context::<KnowledgeSaveRequest>();
    let ComposerDraft(composer_draft) = expect_context::<ComposerDraft>();

    let title = RwSignal::new(String::new());
    let body = RwSignal::new(String::new());
    let tags = RwSignal::new(String::new());
    let initialized = RwSignal::new(false);
    let dirty = RwSignal::new(false);
    let preview = RwSignal::new(false);
    let notice = RwSignal::new(Option::<String>::None);
    let attachment_busy = RwSignal::new(false);
    let stable_document_id = StoredValue::new(document_id.clone());

    let selected = Memo::new({
        let document_id = document_id.clone();
        move |_| {
            documents.with(|items| {
                items
                    .iter()
                    .find(|document| document.id == document_id)
                    .cloned()
            })
        }
    });
    Effect::new(move |_| {
        if initialized.get() {
            return;
        }
        if let Some(document) = selected.get() {
            title.set(document.title);
            body.set(document.body_markdown);
            tags.set(document.tags.join(", "));
            initialized.set(true);
        }
    });

    let save = Callback::new({
        let document_id = document_id.clone();
        move |_: ()| {
            let Some(mut document) = documents.with_untracked(|items| {
                items
                    .iter()
                    .find(|document| document.id == document_id)
                    .cloned()
            }) else {
                return;
            };
            let parsed_tags = tags
                .get_untracked()
                .split(',')
                .map(str::to_string)
                .collect();
            let original = document.clone();
            if document.revise(
                title.get_untracked(),
                body.get_untracked(),
                parsed_tags,
                crate::clock::now_ms(),
            ) {
                replace_document(set_documents, document.clone());
                storage::persist_knowledge_document(Some(original), document);
            }
            dirty.set(false);
            notice.set(None);
        }
    });
    Effect::new(move |_| {
        if save_request.take() {
            save.run(());
        }
    });

    let link_options = Memo::new({
        let document_id = document_id.clone();
        move |_| {
            let mut options = documents.with(|items| {
                items
                    .iter()
                    .filter(|item| item.id != document_id)
                    .map(|item| {
                        link_option(
                            KnowledgeLinkKind::Document,
                            item.id.clone(),
                            item.title.clone(),
                        )
                    })
                    .collect::<Vec<_>>()
            });
            work_items.with(|items| {
                options.extend(items.iter().map(|item| {
                    link_option(KnowledgeLinkKind::Task, item.id.clone(), item.title.clone())
                }))
            });
            projects.with(|items| {
                options.extend(items.iter().map(|item| {
                    link_option(
                        KnowledgeLinkKind::Project,
                        item.id.clone(),
                        item.name.clone(),
                    )
                }))
            });
            meetings.with(|items| {
                options.extend(items.iter().map(|item| {
                    link_option(
                        KnowledgeLinkKind::Meeting,
                        item.id.clone(),
                        item.title.clone(),
                    )
                }))
            });
            people.with(|items| {
                options.extend(items.iter().map(|item| {
                    link_option(
                        KnowledgeLinkKind::Person,
                        item.id.clone(),
                        item.display_name.clone(),
                    )
                }))
            });
            options
        }
    });
    let citation_options = Memo::new(move |_| {
        clips.with(|items| {
            items
                .iter()
                .map(|clip| {
                    DropdownOption::new(clip.id.clone(), clip.title.clone()).in_group("Sources")
                })
                .collect::<Vec<_>>()
        })
    });

    let add_link = {
        let document_id = document_id.clone();
        Callback::new(move |value: String| {
            let Some(option) = link_options
                .with_untracked(|items| items.iter().find(|option| option.value == value).cloned())
            else {
                return;
            };
            let Some((kind, target_id)) = parse_link_value(&value) else {
                return;
            };
            let Some(mut document) = documents
                .with_untracked(|items| items.iter().find(|item| item.id == document_id).cloned())
            else {
                return;
            };
            let mut links = document.links.clone();
            if links
                .iter()
                .any(|link| link.kind == kind && link.target_id == target_id)
            {
                return;
            }
            links.push(KnowledgeLink {
                kind,
                target_id,
                label: option.label,
            });
            let original = document.clone();
            document.update_links(links, crate::clock::now_ms());
            replace_document(set_documents, document.clone());
            storage::persist_knowledge_document(Some(original), document);
        })
    };
    let add_citation = {
        let document_id = document_id.clone();
        Callback::new(move |clip_id: String| {
            let Some(mut document) = documents
                .with_untracked(|items| items.iter().find(|item| item.id == document_id).cloned())
            else {
                return;
            };
            let mut citations = document.citations.clone();
            if citations.iter().any(|citation| citation.clip_id == clip_id) {
                return;
            }
            citations.push(KnowledgeCitation {
                clip_id,
                locator: String::new(),
                note: String::new(),
            });
            let original = document.clone();
            document.update_citations(citations, crate::clock::now_ms());
            replace_document(set_documents, document.clone());
            storage::persist_knowledge_document(Some(original), document);
        })
    };

    let on_attachment = Callback::new({
        let document_id = document_id.clone();
        move |event: web_sys::Event| {
            let Some(file) = event
                .target()
                .and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok())
                .and_then(|input| input.files())
                .and_then(|files| files.get(0))
            else {
                return;
            };
            if file.size().to_u64_saturating() > MAX_ATTACHMENT_BYTES {
                notice.set(Some("Attachments must be 10 MB or smaller.".to_string()));
                return;
            }
            let name = file.name();
            let mime_type = file.type_();
            notice.set(None);
            attachment_busy.set(true);
            let document_id = document_id.clone();
            leptos::task::spawn_local(async move {
                let result = async {
                    let buffer = wasm_bindgen_futures::JsFuture::from(file.array_buffer())
                        .await
                        .map_err(|_| "Could not read that file.".to_string())?;
                    let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
                    let attachment = KnowledgeAttachment {
                        id: uuid::Uuid::new_v4().to_string(),
                        document_id: document_id.clone(),
                        file_name: name,
                        mime_type,
                        bytes,
                        created_at_ms: crate::clock::now_ms(),
                    };
                    let Some(mut document) = documents.with_untracked(|items| {
                        items.iter().find(|item| item.id == document_id).cloned()
                    }) else {
                        return Err("Document no longer exists.".to_string());
                    };
                    let original = document.clone();
                    let mut attachments = document.attachments.clone();
                    attachments.push(attachment.metadata());
                    document.update_attachments(attachments, crate::clock::now_ms());
                    replace_document(set_documents, document.clone());
                    saves.attach_document(original, document, attachment);
                    Ok(())
                }
                .await;
                // The read can finish after navigation; the app-owned writer
                // survives the editor, but its local feedback signals may not.
                attachment_busy.try_set(false);
                notice.try_set(result.err());
            });
        }
    });

    view! {
        <div class="h-full overflow-auto bg-longan-950 text-rambutan-300">
            <div class="mx-auto w-full max-w-6xl px-4 py-4 sm:px-6">
                <header class="mb-4 flex flex-wrap items-center justify-between gap-3">
                    <button type="button" class="rounded-seed px-2 py-1 text-label text-rambutan-400 inset-ring inset-ring-longan-700 hover:text-rambutan-200 cursor-pointer" on:click=move |_| set_active_view.set(AppView::Knowledge)>"← Knowledge"</button>
                    <div class="flex items-center gap-2">
                        <button type="button" class="rounded-grape px-3 py-2 text-label text-blueberry-300 inset-ring inset-ring-longan-700 hover:bg-blueberry-500/10 cursor-pointer" on:click={
                            let document_id = document_id.clone();
                            move |_| {
                                let Some(document) = documents.with_untracked(|items| items.iter().find(|item| item.id == document_id).cloned()) else { return; };
                                composer_draft.set(clips.with_untracked(|items| bruhs_models::document_agent_prompt(&document, items)));
                                set_active_view.set(AppView::Chat);
                            }
                        }>"Ask agent"</button>
                        <button type="button" class="rounded-grape px-3 py-2 text-label text-guava-300 hover:bg-guava-500/10 cursor-pointer" on:click={
                            move |_| {
                                let Some(original) = documents.with_untracked(|items| items.iter().find(|item| item.id == document_id).cloned()) else { return; };
                                set_documents.update(|items| items.retain(|item| item.id != document_id));
                                storage::forget_knowledge_document(original);
                                set_active_view.set(AppView::Knowledge);
                            }
                        }>"Delete"</button>
                        <button type="button" aria-pressed=move || preview.get().to_string() class="rounded-grape px-3 py-2 text-label text-rambutan-300 inset-ring inset-ring-longan-700 hover:bg-longan-800 cursor-pointer" on:click=move |_| preview.update(|value| *value = !*value)>{move || if preview.get() { "Edit" } else { "Preview" }}</button>
                        <button type="button" disabled=move || !dirty.get() class="rounded-grape bg-persimmon-500 px-3 py-2 text-label text-longan-950 hover:bg-persimmon-400 disabled:opacity-40 cursor-pointer" on:click=move |_| save.run(())>"Save"</button>
                    </div>
                </header>

                <Show
                    when=move || initialized.get()
                    fallback=move || view! { <div class="rounded-grape bg-longan-900/30 p-4 text-body-sm text-rambutan-500 inset-ring inset-ring-longan-800">{move || if executor.ready.get() { "Document not found." } else if executor.load_error.get().is_some() { "Document could not be loaded." } else { "Loading document…" }}</div> }
                >
                        <div class="grid min-w-0 gap-5 lg:grid-cols-[minmax(0,1fr)_18rem]">
                            <div class="min-w-0 space-y-3">
                                <Input aria_label="Document title" value=Signal::derive(move || title.get()) on_input=Callback::new(move |value| { title.set(value); dirty.set(true); }) autofocus=true class="text-title".to_string() />
                                <Show when=move || !preview.get() fallback=move || view! { <article class="prose-msg min-h-80 rounded-grape bg-longan-900/30 p-4 text-body text-rambutan-200 inset-ring inset-ring-longan-800" inner_html=move || crate::markdown::render_untrusted(&body.get())></article> }>
                                    <Textarea aria_label="Document body" rows=22 value=Signal::derive(move || body.get()) on_input=Callback::new(move |value| { body.set(value); dirty.set(true); }) placeholder="Write Markdown…".to_string() class="min-h-96 font-mono".to_string() />
                                </Show>
                                <div class="space-y-1"><label class="text-label-sm text-rambutan-500">"Tags — comma separated"</label><Input aria_label="Document tags" value=Signal::derive(move || tags.get()) on_input=Callback::new(move |value| { tags.set(value); dirty.set(true); }) placeholder="research, product, decision".to_string() /></div>
                                <p class="min-h-5 text-label-sm text-rambutan-500" aria-live="polite">{move || {
                                    use crate::durable_metadata::SaveState;
                                    match saves.record_status(crate::task_saves::Kind::Document, &stable_document_id.get_value()) {
                                        SaveState::Error { .. } => "Document or attachment not saved. Your changes are kept; use the save recovery notice to retry.".to_string(),
                                        SaveState::Saving | SaveState::Loading => "Saving document and attachments…".to_string(),
                                        SaveState::Saved => notice.get().unwrap_or_else(|| if dirty.get() { "Unsaved changes".to_string() } else if attachment_busy.get() { "Reading attachment…".to_string() } else { "Saved locally".to_string() }),
                                    }
                                }}</p>
                            </div>
                            <aside class="min-w-0 space-y-4">
                                <KnowledgePanel title="Connections">
                                    <RecordPicker options=Signal::derive(move || link_options.get()) on_change=add_link label="Connect a record" search_label="Search records" placeholder="Connect task, meeting, person…" />
                                    <div class="space-y-1">{move || selected.get().map(|document| document.links.into_iter().map(|link| {
                                        let document_id = stable_document_id.get_value();
                                        let remove = link.clone();
                                        view! {
                                            <button
                                                type="button"
                                                title="Remove connection"
                                                class="flex w-full items-center justify-between gap-2 rounded-seed px-2 py-1.5 text-left hover:bg-longan-800/60 cursor-pointer"
                                                on:click=move |_| edit_document(documents, set_documents, &document_id, |document, now| {
                                                    let mut links = document.links.clone();
                                                    links.retain(|item| item != &remove);
                                                    document.update_links(links, now);
                                                })
                                            >
                                                <span class="min-w-0 truncate text-body-sm text-rambutan-300">{link.label}</span>
                                                <span class="shrink-0 text-label-sm text-rambutan-500">{link.kind.label()}</span>
                                            </button>
                                        }
                                    }).collect_view())}</div>
                                </KnowledgePanel>

                                <KnowledgePanel title="Citations">
                                    <RecordPicker options=Signal::derive(move || citation_options.get()) on_change=add_citation label="Cite a saved source" search_label="Search sources" placeholder="Add source…" />
                                    <div class="space-y-1">{move || selected.get().map(|document| document.citations.into_iter().filter_map(|citation| {
                                        let clip = clips.with(|items| items.iter().find(|clip| clip.id == citation.clip_id).cloned())?;
                                        let document_id = stable_document_id.get_value();
                                        let remove_id = citation.clip_id;
                                        Some(view! {
                                            <button
                                                type="button"
                                                title="Remove citation"
                                                class="block w-full rounded-seed px-2 py-1.5 text-left hover:bg-longan-800/60 cursor-pointer"
                                                on:click=move |_| edit_document(documents, set_documents, &document_id, |document, now| {
                                                    let mut citations = document.citations.clone();
                                                    citations.retain(|item| item.clip_id != remove_id);
                                                    document.update_citations(citations, now);
                                                })
                                            >
                                                <span class="block truncate text-body-sm text-rambutan-300">{clip.title}</span>
                                                <span class="block truncate text-label-sm text-blueberry-300/80">{source_host(&clip.source_url)}</span>
                                            </button>
                                        })
                                    }).collect_view())}</div>
                                </KnowledgePanel>

                                <KnowledgePanel title="Attachments">
                                    <label class="block rounded-grape px-3 py-2 text-center text-label text-rambutan-300 inset-ring inset-ring-longan-700 hover:bg-longan-800/60 cursor-pointer">
                                        <input type="file" class="sr-only" disabled=move || attachment_busy.get() on:change=move |event| on_attachment.run(event) />
                                        {move || if attachment_busy.get() { "Reading…" } else { "Attach local file" }}
                                    </label>
                                    <div class="space-y-1">{move || selected.get().map(|document| document.attachments.into_iter().map(|attachment| {
                                        let attachment_id = attachment.id.clone();
                                        view! {
                                            <button
                                                type="button"
                                                title="Download attachment"
                                                class="block w-full rounded-seed bg-longan-900/50 px-2 py-1.5 text-left hover:bg-longan-800/70 cursor-pointer"
                                                on:click=move |_| download_attachment(attachment_id.clone())
                                            >
                                                <span class="block truncate text-body-sm text-rambutan-300">{attachment.file_name}</span>
                                                <span class="text-label-sm tabular-nums text-rambutan-500">{format_bytes(attachment.size_bytes)}</span>
                                            </button>
                                        }
                                    }).collect_view())}</div>
                                </KnowledgePanel>

                                <KnowledgePanel title="Backlinks">
                                    <div class="space-y-1">{move || {
                                        let items = documents.with(|items| backlinks_for(&stable_document_id.get_value(), items));
                                        if items.is_empty() {
                                            return view! {
                                                <p class="px-2 text-body-sm text-rambutan-500">"Nothing links here yet."</p>
                                            }
                                            .into_any();
                                        }
                                        let rows = items.into_iter().map(|backlink| {
                                            let id = backlink.document_id;
                                            view! {
                                                <button
                                                    type="button"
                                                    class="block w-full truncate rounded-seed px-2 py-1.5 text-left text-body-sm text-blueberry-300 hover:bg-longan-800/60 cursor-pointer"
                                                    on:click=move |_| set_active_view.set(AppView::DocumentDetail { document_id: id.clone() })
                                                >
                                                    {backlink.document_title}
                                                </button>
                                            }
                                        }).collect_view();
                                        view! { <div>{rows}</div> }.into_any()
                                    }}</div>
                                </KnowledgePanel>

                                <KnowledgePanel title="History">
                                    <div class="max-h-48 space-y-1 overflow-auto">{move || selected.get().map(|document| document.versions.into_iter().map(|version| {
                                        let document_id = stable_document_id.get_value();
                                        let version_id = version.id.clone();
                                        view! { <button type="button" class="block w-full rounded-seed px-2 py-1.5 text-left hover:bg-longan-800/60 cursor-pointer" on:click=move |_| {
                                            let Some(mut document) = documents.with_untracked(|items| items.iter().find(|item| item.id == document_id).cloned()) else { return; };
                                            let original = document.clone();
                                            if document.restore_version(&version_id, crate::clock::now_ms()) {
                                                title.set(document.title.clone()); body.set(document.body_markdown.clone()); tags.set(document.tags.join(", ")); dirty.set(false);
                                                replace_document(set_documents, document.clone()); storage::persist_knowledge_document(Some(original), document); notice.set(None);
                                            }
                                        }><span class="block truncate text-body-sm text-rambutan-300">{version.title}</span><span class="text-label-sm text-rambutan-500">{format_timestamp(version.recorded_at_ms)}</span></button> }
                                    }).collect_view())}</div>
                                </KnowledgePanel>
                            </aside>
                        </div>
                </Show>
            </div>
        </div>
    }
}

#[component]
fn KnowledgePanel(title: &'static str, children: Children) -> impl IntoView {
    view! {
        <section class="space-y-2 rounded-grape bg-longan-900/40 p-3 inset-ring inset-ring-longan-800">
            <h2 class="text-label-sm uppercase tracking-[0.12em] text-rambutan-500">{title}</h2>
            {children()}
        </section>
    }
}

#[component]
fn ClipEditor(clip_id: String) -> impl IntoView {
    let executor = expect_context::<crate::workflow_executor::WorkflowExecutor>();
    let saves = expect_context::<crate::task_saves::TaskSaves>();
    let stable_clip_id = StoredValue::new(clip_id.clone());
    let SetKnowledgeDocumentsSignal(set_documents) =
        expect_context::<SetKnowledgeDocumentsSignal>();
    let SourceClipsSignal(clips) = expect_context::<SourceClipsSignal>();
    let SetSourceClipsSignal(set_clips) = expect_context::<SetSourceClipsSignal>();
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    let ComposerDraft(composer_draft) = expect_context::<ComposerDraft>();
    let selected = Memo::new({
        let clip_id = clip_id.clone();
        move |_| clips.with(|items| items.iter().find(|clip| clip.id == clip_id).cloned())
    });
    let title = RwSignal::new(String::new());
    let url = RwSignal::new(String::new());
    let source_name = RwSignal::new(String::new());
    let author = RwSignal::new(String::new());
    let published = RwSignal::new(String::new());
    let excerpt = RwSignal::new(String::new());
    let tags = RwSignal::new(String::new());
    let error = RwSignal::new(Option::<String>::None);
    let initialized = RwSignal::new(false);
    let dirty = RwSignal::new(false);
    Effect::new(move |_| {
        if !initialized.get() {
            if let Some(clip) = selected.get() {
                title.set(clip.title);
                url.set(clip.source_url);
                source_name.set(clip.source_name);
                author.set(clip.author);
                published.set(clip.published_on.unwrap_or_default());
                excerpt.set(clip.excerpt);
                tags.set(clip.tags.join(", "));
                initialized.set(true);
            }
        }
    });
    let save = {
        let clip_id = clip_id.clone();
        move |_| {
            let next_title = title.get_untracked().trim().to_string();
            let next_url = url.get_untracked().trim().to_string();
            if next_title.is_empty() || next_url.is_empty() {
                error.set(Some("A title and source URL are required.".to_string()));
                return;
            }
            let Some(mut clip) =
                clips.with_untracked(|items| items.iter().find(|clip| clip.id == clip_id).cloned())
            else {
                return;
            };
            let original = clip.clone();
            clip.title = next_title;
            clip.source_url = next_url;
            clip.source_name = source_name.get_untracked().trim().to_string();
            clip.author = author.get_untracked().trim().to_string();
            clip.published_on = (!published.get_untracked().trim().is_empty())
                .then(|| published.get_untracked().trim().to_string());
            clip.excerpt = excerpt.get_untracked();
            clip.tags = bruhs_models::normalize_tags(
                tags.get_untracked()
                    .split(',')
                    .map(str::to_string)
                    .collect(),
            );
            clip.updated_at_ms = crate::clock::now_ms();
            set_clips.update(|items| {
                if let Some(current) = items.iter_mut().find(|item| item.id == clip.id) {
                    *current = clip.clone();
                }
            });
            storage::persist_source_clip(Some(original), clip);
            dirty.set(false);
            error.set(None);
        }
    };
    let delete = Callback::new({
        let clip_id = clip_id.clone();
        move |_| {
            let Some(original_source) =
                clips.with_untracked(|items| items.iter().find(|clip| clip.id == clip_id).cloned())
            else {
                return;
            };
            set_clips.update(|items| items.retain(|clip| clip.id != clip_id));
            let mut changed_documents = Vec::new();
            set_documents.update(|documents| {
                for document in documents {
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
                        let original = document.clone();
                        document.update_citations(citations, crate::clock::now_ms());
                        changed_documents.push((original, document.clone()));
                    }
                }
            });
            saves.delete_source(original_source, changed_documents);
            set_active_view.set(AppView::Knowledge);
        }
    });
    let ask_agent = move |_| {
        let Some(clip) =
            clips.with_untracked(|items| items.iter().find(|item| item.id == clip_id).cloned())
        else {
            return;
        };
        composer_draft.set(bruhs_models::source_clip_agent_prompt(&clip));
        set_active_view.set(AppView::Chat);
    };
    view! {
        <div class="h-full overflow-auto bg-longan-950 text-rambutan-300"><div class="mx-auto w-full max-w-3xl space-y-4 px-4 py-4 sm:px-6">
            <header class="flex items-center justify-between gap-3">
                <button
                    type="button"
                    class="rounded-seed px-2 py-1 text-label text-rambutan-400 inset-ring inset-ring-longan-700 hover:text-rambutan-200 cursor-pointer"
                    on:click=move |_| set_active_view.set(AppView::Knowledge)
                >
                    "← Knowledge"
                </button>
                <div class="flex items-center gap-2">
                    <button
                        type="button"
                        class="rounded-grape px-3 py-2 text-label text-blueberry-300 inset-ring inset-ring-longan-700 hover:bg-blueberry-500/10 cursor-pointer"
                        on:click=ask_agent
                    >
                        "Ask agent"
                    </button>
                    <button
                        type="button"
                        disabled=move || !initialized.get() || !dirty.get()
                        class="rounded-grape bg-persimmon-500 px-3 py-2 text-label text-longan-950 hover:bg-persimmon-400 disabled:opacity-40 cursor-pointer"
                        on:click=save
                    >
                        "Save source"
                    </button>
                </div>
            </header>
            <Show when=move || initialized.get() fallback=move || view! { <p class="rounded-grape bg-longan-900/30 p-4 text-body-sm text-rambutan-500 inset-ring inset-ring-longan-800">{move || if executor.ready.get() { "Source not found." } else if executor.load_error.get().is_some() { "Source could not be loaded." } else { "Loading source…" }}</p> }>
            <div class="space-y-3 rounded-grape bg-longan-900/40 p-4 inset-ring inset-ring-longan-800">
                {source_field("Title", "Source title", None, title, dirty, true, "")}
                {source_field("Source URL", "Source URL", Some("url"), url, dirty, false, "")}
                <Show when=move || error.get().is_some()>
                    <p role="alert" class="text-label text-guava-300">{move || error.get().unwrap_or_default()}</p>
                </Show>
                <div class="grid gap-3 sm:grid-cols-2">
                    {source_field("Publication", "Publication", None, source_name, dirty, false, "")}
                    {source_field("Author", "Author", None, author, dirty, false, "")}
                </div>
                <div class="grid gap-3 sm:grid-cols-2">
                    {source_field("Published", "Published", Some("date"), published, dirty, false, "")}
                    {source_field("Tags", "Source tags", None, tags, dirty, false, "research, product")}
                </div>
                <div class="space-y-1">
                    <label class="text-label-sm text-rambutan-500">"Excerpt or note"</label>
                    <Textarea
                        aria_label="Excerpt or note"
                        rows=12
                        value=Signal::derive(move || excerpt.get())
                        on_input=Callback::new(move |value| {
                            excerpt.set(value);
                            dirty.set(true);
                        })
                    />
                </div>
                <Show when=move || !url.get().trim().is_empty()><a href=move || crate::markdown::safe_url(&url.get()).map(str::to_owned) target="_blank" rel="noopener noreferrer" class="inline-flex items-center gap-1 text-label text-blueberry-300 hover:text-blueberry-200">"Open original" <span aria-hidden="true">"↗"</span></a></Show>
                <p role="status" class="text-label-sm text-rambutan-500">{move || {
                    use crate::durable_metadata::SaveState;
                    match saves.record_status(crate::task_saves::Kind::SourceClip, &stable_clip_id.get_value()) {
                        SaveState::Error { .. } => "Source not saved. Your changes are kept; use the save recovery notice to retry.",
                        SaveState::Saving | SaveState::Loading => "Saving source…",
                        SaveState::Saved if dirty.get() => "Unsaved changes",
                        SaveState::Saved => "Saved locally",
                    }
                }}</p>
            </div>
            <button type="button" class="text-label text-guava-300 hover:text-guava-200 cursor-pointer" on:click=move |_| delete.run(())>"Delete source"</button>
            </Show>
        </div></div>
    }
}

/// One captioned field of the source editor; typing marks the source dirty.
fn source_field(
    caption: &'static str,
    label: &'static str,
    input_type: Option<&'static str>,
    value: RwSignal<String>,
    dirty: RwSignal<bool>,
    autofocus: bool,
    placeholder: &'static str,
) -> impl IntoView {
    let input = match input_type {
        Some(kind) => view! {
            <Input
                aria_label=label
                input_type=kind.to_string()
                value=Signal::derive(move || value.get())
                on_input=Callback::new(move |text| {
                    value.set(text);
                    dirty.set(true);
                })
                autofocus=autofocus
                placeholder=placeholder.to_string()
            />
        }
        .into_any(),
        None => view! {
            <Input
                aria_label=label
                value=Signal::derive(move || value.get())
                on_input=Callback::new(move |text| {
                    value.set(text);
                    dirty.set(true);
                })
                autofocus=autofocus
                placeholder=placeholder.to_string()
            />
        }
        .into_any(),
    };
    view! {
        <div class="space-y-1">
            <label class="text-label-sm text-rambutan-500">{caption}</label>
            {input}
        </div>
    }
}

/// Apply `change` to the stored document `id` (stamped now) and save it
/// against the version it was read from.
fn edit_document(
    documents: ReadSignal<Vec<KnowledgeDocument>>,
    set_documents: WriteSignal<Vec<KnowledgeDocument>>,
    id: &str,
    change: impl FnOnce(&mut KnowledgeDocument, i64),
) {
    let Some(mut document) =
        documents.with_untracked(|items| items.iter().find(|item| item.id == id).cloned())
    else {
        return;
    };
    let original = document.clone();
    change(&mut document, crate::clock::now_ms());
    replace_document(set_documents, document.clone());
    storage::persist_knowledge_document(Some(original), document);
}

fn replace_document(
    set_documents: WriteSignal<Vec<KnowledgeDocument>>,
    document: KnowledgeDocument,
) {
    set_documents.update(|items| {
        if let Some(current) = items.iter_mut().find(|item| item.id == document.id) {
            *current = document;
        }
    });
}

fn link_option(kind: KnowledgeLinkKind, id: String, label: String) -> DropdownOption {
    DropdownOption::new(format!("{}|{id}", link_kind_key(kind)), label).in_group(kind.label())
}

fn link_kind_key(kind: KnowledgeLinkKind) -> &'static str {
    match kind {
        KnowledgeLinkKind::Document => "document",
        KnowledgeLinkKind::Task => "task",
        KnowledgeLinkKind::Project => "project",
        KnowledgeLinkKind::Meeting => "meeting",
        KnowledgeLinkKind::Person => "person",
    }
}

fn parse_link_value(value: &str) -> Option<(KnowledgeLinkKind, String)> {
    let (kind, id) = value.split_once('|')?;
    let kind = match kind {
        "document" => KnowledgeLinkKind::Document,
        "task" => KnowledgeLinkKind::Task,
        "project" => KnowledgeLinkKind::Project,
        "meeting" => KnowledgeLinkKind::Meeting,
        "person" => KnowledgeLinkKind::Person,
        _ => return None,
    };
    Some((kind, id.to_string()))
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

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes.to_f64_lossy() / 1_048_576.0)
    } else if bytes >= 1_024 {
        format!("{:.1} KB", bytes.to_f64_lossy() / 1_024.0)
    } else {
        format!("{bytes} B")
    }
}

fn format_timestamp(timestamp_ms: i64) -> String {
    js_sys::Date::new(&JsValue::from_f64(
        bruhs_models::EpochMs::saturating_from_i64(timestamp_ms).as_f64(),
    ))
    .to_locale_string("en-US", &JsValue::UNDEFINED)
    .as_string()
    .unwrap_or_else(|| "Earlier".to_string())
}

fn download_attachment(id: String) {
    let pending = expect_context::<crate::task_saves::TaskSaves>().pending_attachment(&id);
    leptos::task::spawn_local(async move {
        let attachment = if let Some(pending) = pending {
            pending
        } else {
            let Ok(Some(attachment)) = storage::load_knowledge_attachment(&id).await else {
                return;
            };
            std::sync::Arc::new(attachment)
        };
        let bytes = js_sys::Uint8Array::from(attachment.bytes.as_slice());
        let parts = js_sys::Array::new();
        parts.push(&bytes.buffer());
        let Ok(blob) = web_sys::Blob::new_with_u8_array_sequence(&parts) else {
            return;
        };
        let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else {
            return;
        };
        let Some(document) = web_sys::window().and_then(|window| window.document()) else {
            let _ = web_sys::Url::revoke_object_url(&url);
            return;
        };
        if let Some(anchor) = document
            .create_element("a")
            .ok()
            .and_then(|element| element.dyn_into::<web_sys::HtmlAnchorElement>().ok())
        {
            anchor.set_href(&url);
            anchor.set_download(&attachment.file_name);
            anchor.click();
        }
        let _ = web_sys::Url::revoke_object_url(&url);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_values_round_trip_every_domain() {
        for kind in [
            KnowledgeLinkKind::Document,
            KnowledgeLinkKind::Task,
            KnowledgeLinkKind::Project,
            KnowledgeLinkKind::Meeting,
            KnowledgeLinkKind::Person,
        ] {
            let value = link_option(kind, "id".into(), "Label".into()).value;
            assert_eq!(parse_link_value(&value), Some((kind, "id".into())));
        }
    }

    #[test]
    fn previews_strip_heading_markers() {
        assert_eq!(document_preview("\n## Decision\nBody"), "Decision");
        assert_eq!(document_preview(""), "Empty document");
    }
}
