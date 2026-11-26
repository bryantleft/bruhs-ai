use std::sync::Arc;

use bruhs_models::api::QuestionUpload;
use bruhs_models::logic::attachments::{
    admit, kind_for_name, upload_label, FileKind, COULD_NOT_READ, MAX_FILES_PER_MESSAGE,
    TOO_MANY_FILES,
};
use bruhs_models::num::ToIntSaturating;
use leptos::prelude::*;
use uuid::Uuid;
use wasm_bindgen::JsCast;

use crate::api::{remove_question_file, upload_question_file, ApiError};
use crate::components::file_attachments::OfferPhase;
use crate::state::{PermissionRequest, QuestionDraft, QuestionFileOffer};
use crate::ui::{Button, ButtonSize, ButtonVariant, Icon};

async fn send_file(draft: ArcRwSignal<QuestionDraft>, upload: QuestionUpload, bytes: Arc<[u8]>) {
    let result = upload_question_file(&upload, &bytes).await;
    draft.update(|draft| {
        if let Some(tile) = draft
            .uploads
            .iter_mut()
            .find(|tile| tile.upload.upload_id == upload.upload_id)
        {
            match result {
                Ok(receipt) => {
                    tile.phase = OfferPhase::Ready { path: receipt.path };
                    tile.bytes = None;
                    draft.answers[upload.question]
                        .attachments
                        .push(upload.upload_id);
                }
                Err(error) => {
                    tile.phase = OfferPhase::Failed {
                        message: error.to_string(),
                        retry: true,
                    };
                    tile.bytes = Some(bytes);
                }
            }
        }
    });
}

fn enqueue(
    draft: ArcRwSignal<QuestionDraft>,
    request: &PermissionRequest,
    question: usize,
    input: web_sys::File,
) -> Result<(), &'static str> {
    let name = upload_label(&input.name());
    let size = input.size().to_u64_saturating();
    let thread_id = request
        .thread_id
        .parse()
        .map_err(|_| "This thread could not be identified.")?;
    let upload = QuestionUpload {
        thread_id,
        request_id: request.id.clone(),
        question,
        upload_id: Uuid::new_v4(),
        name,
    };
    draft
        .try_update(|state| {
            if state.uploads.len() >= MAX_FILES_PER_MESSAGE {
                return Err(TOO_MANY_FILES);
            }
            let image_bytes = state
                .uploads
                .iter()
                .filter(|tile| kind_for_name(&tile.upload.name) == FileKind::Image)
                .map(|tile| tile.size)
                .sum();
            admit(state.uploads.len(), image_bytes, &upload.name, size)?;
            state.uploads.push(QuestionFileOffer {
                upload: upload.clone(),
                size,
                phase: OfferPhase::Pending,
                bytes: None,
            });
            Ok(())
        })
        .ok_or("This question is no longer open.")??;
    leptos::task::spawn_local(async move {
        let buffer = wasm_bindgen_futures::JsFuture::from(input.array_buffer()).await;
        if let Ok(buffer) = buffer {
            let bytes = Arc::from(js_sys::Uint8Array::new(&buffer).to_vec());
            send_file(draft, upload, bytes).await;
        } else {
            draft.update(|draft| {
                if let Some(tile) = draft.uploads.iter_mut().find(|tile| tile.upload == upload) {
                    tile.phase = OfferPhase::Failed {
                        message: COULD_NOT_READ.into(),
                        retry: false,
                    };
                }
            });
        }
    });
    Ok(())
}

#[component]
pub(super) fn QuestionFiles(
    question: usize,
    request: PermissionRequest,
    draft: ArcRwSignal<QuestionDraft>,
    busy: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    textarea: AnyView,
) -> impl IntoView {
    let state = StoredValue::new((request, draft));
    let picker: NodeRef<leptos::html::Input> = NodeRef::new();
    let accept = move |files: web_sys::FileList| {
        if busy.get_untracked() {
            return;
        }
        error.set(None);
        state.with_value(|(request, draft)| {
            for index in 0..files.length() {
                if let Some(file) = files.get(index) {
                    if let Err(message) = enqueue(draft.clone(), request, question, file) {
                        error.set(Some(message.into()));
                    }
                }
            }
        });
    };
    let retry = move |id| {
        state.with_value(|(_, draft)| {
            let pending = draft
                .try_update(|draft| {
                    let item = draft
                        .uploads
                        .iter_mut()
                        .find(|tile| tile.upload.upload_id == id)?;
                    let bytes = item.bytes.take()?;
                    item.phase = OfferPhase::Pending;
                    Some((item.upload.clone(), bytes))
                })
                .flatten();
            if let Some((upload, bytes)) = pending {
                leptos::task::spawn_local(send_file(draft.clone(), upload, bytes));
            }
        });
    };
    let remove = move |id| {
        let draft = state.with_value(|(_, draft)| draft.clone());
        let previous = draft
            .try_update(|draft| {
                let tile = draft
                    .uploads
                    .iter_mut()
                    .find(|tile| tile.upload.upload_id == id)?;
                let phase = std::mem::replace(&mut tile.phase, OfferPhase::Pending);
                Some((tile.upload.clone(), phase))
            })
            .flatten();
        if let Some((upload, previous)) = previous {
            leptos::task::spawn_local(async move {
                let result = remove_question_file(&upload).await;
                let removed =
                    result.is_ok() || matches!(&result, Err(ApiError::Http { status: 404, .. }));
                draft.update(|draft| {
                    if removed {
                        draft.uploads.retain(|tile| tile.upload.upload_id != id);
                        draft.answers[question]
                            .attachments
                            .retain(|item| *item != id);
                    } else if let Some(tile) = draft
                        .uploads
                        .iter_mut()
                        .find(|tile| tile.upload.upload_id == id)
                    {
                        tile.phase = previous;
                    }
                });
                if let Err(failure) = result {
                    if !removed {
                        error.try_set(Some(failure.to_string()));
                    }
                }
            });
        }
    };
    view! {
        <div on:paste=move |event: web_sys::ClipboardEvent| {
            if let Some(files) = event.clipboard_data().and_then(|data| data.files()).filter(|files| files.length() > 0) {
                event.prevent_default();
                event.stop_propagation();
                accept(files);
            }
        }
        on:dragenter=move |event: web_sys::DragEvent| {
            if super::super::transfer_has_files(event.data_transfer().as_ref()) {
                event.stop_propagation();
            }
        }
        on:dragleave=move |event: web_sys::DragEvent| {
            if super::super::transfer_has_files(event.data_transfer().as_ref()) {
                event.stop_propagation();
            }
        }
        on:dragover=move |event: web_sys::DragEvent| {
            if super::super::transfer_has_files(event.data_transfer().as_ref()) {
                event.prevent_default();
                event.stop_propagation();
            }
        }
        on:drop=move |event: web_sys::DragEvent| {
            if let Some(files) = event.data_transfer().and_then(|data| data.files()).filter(|files| files.length() > 0) {
                event.prevent_default();
                event.stop_propagation();
                accept(files);
            }
        }>
            {textarea}
            <input node_ref=picker type="file" multiple=true class="hidden" on:change=move |event| {
                if let Some(input) = event.target().and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok()) {
                    if let Some(files) = input.files() {
                        accept(files);
                    }
                    input.set_value("");
                }
            }/>
            <div class="mt-1 flex flex-wrap items-center gap-1.5">
                <Button variant=ButtonVariant::Ghost size=ButtonSize::Small disabled=busy
                    on_click=Callback::new(move |_| { if let Some(input) = picker.get() { input.click(); } })>
                    <Icon name="paperclip" size=12 />
                    "Attach files"
                </Button>
                {move || {
                    let items = state.with_value(|(_, draft)| draft.with(|draft|
                        draft.uploads.iter().filter(|item| item.upload.question == question)
                            .map(|item| (item.upload.upload_id, item.upload.name.clone(), item.phase.clone()))
                            .collect::<Vec<_>>()
                    ));
                    items.into_iter().map(|(id, name, phase)| {
                        let is_pending = matches!(phase, OfferPhase::Pending);
                        let can_retry = matches!(phase, OfferPhase::Failed { retry: true, .. });
                        let description = match &phase {
                            OfferPhase::Pending => Some("Working".to_string()),
                            OfferPhase::Failed { message, .. } => Some(message.clone()),
                            OfferPhase::Ready { .. } => None,
                        };
                        view! {
                            <div class="flex min-h-control-sm min-w-0 max-w-full flex-wrap items-center gap-1 rounded-grape border border-longan-800 px-2 text-label-sm">
                                <span class="min-w-0 break-all text-rambutan-200">{name}</span>
                                {description.map(|text| view! { <span role="status" class="text-rambutan-400">{text}</span> })}
                                {can_retry.then(|| view! {
                                    <Button variant=ButtonVariant::Ghost size=ButtonSize::Small disabled=busy on_click=Callback::new(move |_| retry(id))>"Retry"</Button>
                                })}
                                <Button variant=ButtonVariant::Ghost size=ButtonSize::Small
                                    disabled=Signal::derive(move || busy.get() || is_pending)
                                    on_click=Callback::new(move |_| remove(id))>"Remove"</Button>
                            </div>
                        }
                    }).collect_view()
                }}
            </div>
        </div>
    }
}
