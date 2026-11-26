//! Composer file tiles and the read-only viewer beside the conversation.
//!
//! A tile appears as soon as a file is added. Bytes upload in the background.
//! Send stays blocked until every tile is ready or removed.

use std::{collections::BTreeMap, rc::Rc};

use bruhs_models::logic::attachments::{
    admit, kind_for_name, upload_label, FileKind, UploadPhase, COULD_NOT_READ, TOO_MANY_FILES,
    UPLOAD_DID_NOT_FINISH,
};
use bruhs_models::num::ToIntSaturating;
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::api::{open_attachment, upload_attachment};
use crate::state::{OpenFile, OpenFileSignal, PanelSurface, RightPanel, RightPanelSignal};
use crate::ui::Icon;

/// In-memory file offers keyed by conversation, including uploads in progress.
#[derive(Copy, Clone)]
pub struct FileOffers {
    active: ReadSignal<Option<String>>,
    lists: RwSignal<BTreeMap<String, Vec<FileOffer>>, LocalStorage>,
    changed: StoredValue<Option<Callback<String>>>,
}

impl FileOffers {
    pub fn new(active: ReadSignal<Option<String>>) -> Self {
        Self {
            active,
            lists: RwSignal::new_local(BTreeMap::new()),
            changed: StoredValue::new(None),
        }
    }

    fn target(self) -> Option<OfferTarget> {
        Some(self.target_for(self.active.get_untracked()?))
    }

    fn target_for(self, id: String) -> OfferTarget {
        OfferTarget {
            id,
            lists: self.lists,
            changed: self.changed,
        }
    }

    pub fn with<R>(self, read: impl FnOnce(&[FileOffer]) -> R) -> R {
        self.active.with(|id| {
            self.lists.with(|lists| {
                read(
                    id.as_ref()
                        .and_then(|id| lists.get(id))
                        .map_or(&[], Vec::as_slice),
                )
            })
        })
    }

    pub fn with_untracked<R>(self, read: impl FnOnce(&[FileOffer]) -> R) -> R {
        match self.target() {
            Some(target) => target.read(read),
            None => read(&[]),
        }
    }

    pub fn try_update<R>(self, edit: impl FnOnce(&mut Vec<FileOffer>) -> R) -> Option<R> {
        let target = self.target()?;
        let result = target
            .lists
            .try_update(|lists| edit(lists.entry(target.id.clone()).or_default()));
        target.notify_changed();
        result
    }

    pub fn set(self, items: Vec<FileOffer>) {
        if let Some(id) = self.active.get_untracked() {
            self.set_for(id, items);
        }
    }

    pub fn set_for(self, id: String, items: Vec<FileOffer>) {
        self.lists.update(|lists| {
            if items.is_empty() {
                lists.remove(&id);
            } else {
                lists.insert(id.clone(), items);
            }
        });
        if let Some(changed) = self.changed.get_value() {
            changed.run(id);
        }
    }

    pub(crate) fn on_change(self, changed: Callback<String>) {
        self.changed.set_value(Some(changed));
    }

    pub(crate) fn snapshot(self, id: &str) -> Vec<SavedFileOffer> {
        self.lists.with_untracked(|lists| {
            lists
                .get(id)
                .into_iter()
                .flatten()
                .map(SavedFileOffer::from)
                .collect()
        })
    }

    pub(crate) fn has_unsaved_uploads(self) -> bool {
        self.lists.with(|lists| {
            lists
                .values()
                .flatten()
                .any(|offer| matches!(offer.phase, OfferPhase::Pending) || offer.bytes.is_some())
        })
    }

    pub fn is_empty_for(self, id: &str) -> bool {
        self.lists
            .with_untracked(|lists| lists.get(id).is_none_or(Vec::is_empty))
    }

    pub fn forget(self, id: &str) {
        self.lists.update(|lists| {
            lists.remove(id);
        });
    }
}

struct OfferTarget {
    id: String,
    lists: RwSignal<BTreeMap<String, Vec<FileOffer>>, LocalStorage>,
    changed: StoredValue<Option<Callback<String>>>,
}

impl OfferTarget {
    fn read<R>(&self, read: impl FnOnce(&[FileOffer]) -> R) -> R {
        self.lists
            .with_untracked(|lists| read(lists.get(&self.id).map_or(&[], Vec::as_slice)))
    }

    fn update(&self, edit: impl FnOnce(&mut Vec<FileOffer>)) {
        self.lists.update(|lists| {
            if let Some(items) = lists.get_mut(&self.id) {
                edit(items);
            }
        });
        self.notify_changed();
    }

    fn push(&self, offer: FileOffer) {
        self.lists
            .update(|lists| lists.entry(self.id.clone()).or_default().push(offer));
        self.notify_changed();
    }

    fn notify_changed(&self) {
        if let Some(changed) = self.changed.try_get_value().flatten() {
            changed.run(self.id.clone());
        }
    }
}

#[derive(Clone)]
pub struct FileOffer {
    pub id: String,
    pub name: String,
    pub kind: FileKind,
    pub declared_bytes: u64,
    pub phase: OfferPhase,
    pub bytes: Option<Rc<Vec<u8>>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OfferPhase {
    Pending,
    Ready { path: String },
    Failed { message: String, retry: bool },
}

/// File data stays with the upload. The draft only saves the file reference
/// or an explicit tile explaining how to recover an interrupted upload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SavedFileOffer {
    id: String,
    name: String,
    kind: FileKind,
    declared_bytes: u64,
    phase: OfferPhase,
}

impl From<&FileOffer> for SavedFileOffer {
    fn from(offer: &FileOffer) -> Self {
        let phase = match &offer.phase {
            OfferPhase::Pending | OfferPhase::Failed { retry: true, .. } => OfferPhase::Failed {
                message: "Remove this attachment and choose the file again.".into(),
                retry: false,
            },
            phase => phase.clone(),
        };
        Self {
            id: offer.id.clone(),
            name: offer.name.clone(),
            kind: offer.kind,
            declared_bytes: offer.declared_bytes,
            phase,
        }
    }
}

impl From<SavedFileOffer> for FileOffer {
    fn from(saved: SavedFileOffer) -> Self {
        Self {
            id: saved.id,
            name: saved.name,
            kind: saved.kind,
            declared_bytes: saved.declared_bytes,
            phase: saved.phase,
            bytes: None,
        }
    }
}

impl FileOffer {
    pub fn upload_phase(&self) -> UploadPhase {
        match self.phase {
            OfferPhase::Pending => UploadPhase::Pending,
            OfferPhase::Ready { .. } => UploadPhase::Ready,
            OfferPhase::Failed { .. } => UploadPhase::Failed,
        }
    }

    /// Admitted files count toward the message limits. A rejection does not.
    pub fn counts(&self) -> bool {
        !matches!(self.phase, OfferPhase::Failed { retry: false, .. })
    }

    pub fn ready_path(&self) -> Option<&str> {
        match &self.phase {
            OfferPhase::Ready { path } => Some(path),
            _ => None,
        }
    }
}

pub fn offers_block_send(offers: &[FileOffer]) -> bool {
    let phases: Vec<UploadPhase> = offers.iter().map(FileOffer::upload_phase).collect();
    bruhs_models::logic::attachments::send_blocked(&phases)
}

/// Add one browser file. `Some` is a notice when the message is already full.
pub fn enqueue_browser_file(files: FileOffers, file: web_sys::File) -> Option<&'static str> {
    enqueue_browser_file_at(files.target()?, file)
}

pub(crate) fn enqueue_browser_file_for(
    files: FileOffers,
    id: &str,
    file: web_sys::File,
) -> Option<&'static str> {
    enqueue_browser_file_at(files.target_for(id.to_owned()), file)
}

pub(super) fn transfer_has_files(transfer: Option<&web_sys::DataTransfer>) -> bool {
    transfer.is_some_and(|transfer| {
        transfer
            .types()
            .iter()
            .any(|kind| kind.as_string().as_deref() == Some("Files"))
    })
}

fn enqueue_browser_file_at(offers: OfferTarget, file: web_sys::File) -> Option<&'static str> {
    if offers.read(|items| items.len() >= bruhs_models::logic::attachments::MAX_FILES_PER_MESSAGE) {
        return Some(TOO_MANY_FILES);
    }
    let name = {
        let raw = file.name();
        if upload_label(&raw).is_empty() {
            "file".to_string()
        } else {
            raw
        }
    };
    let declared = file.size().to_u64_saturating();
    let (count, image_bytes) = offers.read(|items| committed(items, None));
    let id = uuid::Uuid::new_v4().to_string();
    match admit(count, image_bytes, &name, declared) {
        Err(message) => {
            offers.push(rejected(&id, &name, declared, message));
            None
        }
        Ok(kind) => {
            offers.push(FileOffer {
                id: id.clone(),
                name: name.clone(),
                kind,
                declared_bytes: declared,
                phase: OfferPhase::Pending,
                bytes: None,
            });
            leptos::task::spawn_local(async move {
                let read = wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await;
                let Ok(buffer) = read else {
                    mark_failed(&offers, &id, COULD_NOT_READ, false, None);
                    return;
                };
                let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
                finish_upload(offers, id, name, bytes).await;
            });
            None
        }
    }
}

pub fn retry_offer(files: FileOffers, id: String) {
    let Some(offers) = files.target() else {
        return;
    };
    let Some((name, bytes)) = offers.read(|items| {
        items
            .iter()
            .find(|item| item.id == id)
            .and_then(|item| item.bytes.clone().map(|bytes| (item.name.clone(), bytes)))
    }) else {
        return;
    };
    offers.update(|items| {
        if let Some(item) = items.iter_mut().find(|item| item.id == id) {
            item.phase = OfferPhase::Pending;
        }
    });
    leptos::task::spawn_local(async move {
        finish_upload(offers, id, name, (*bytes).clone()).await;
    });
}

pub fn show_open_file(
    open: RwSignal<Option<OpenFile>>,
    panel: RwSignal<RightPanel>,
    path: String,
    label: String,
    project: Option<String>,
    source: bool,
) {
    open.set(Some(OpenFile {
        path,
        project,
        source,
        label,
    }));
    panel.update(|panel| panel.show(PanelSurface::File));
}

#[component]
pub fn FileTiles() -> impl IntoView {
    let offers = expect_context::<FileOffers>();
    let active = expect_context::<crate::state::ActiveThreadId>().0;
    let threads = expect_context::<crate::state::ThreadsSignal>().0;
    let project = Memo::new(move |_| {
        active.get().and_then(|id| {
            threads.with(|rows| {
                rows.iter()
                    .find(|thread| thread.id == id)
                    .and_then(|thread| thread.project_path.clone())
            })
        })
    });
    let OpenFileSignal(open_file) = expect_context::<OpenFileSignal>();
    let RightPanelSignal(panel) = expect_context::<RightPanelSignal>();
    view! {
        {move || {
            let items = offers.with(<[FileOffer]>::to_vec);
            (!items.is_empty()).then(|| view! {
                <div data-file-offers="" class="flex flex-wrap items-center gap-1.5 border-b border-longan-800/60 px-3 pt-2.5 pb-2">
                    {items.into_iter().map(|item| {
                        let id = item.id.clone();
                        let remove_id = id.clone();
                        let retry_id = id;
                        let name = item.name.clone();
                        let phase_name = phase_name(&item.phase);
                        let ready = item.ready_path().map(str::to_string);
                        let can_open = ready.is_some();
                        let is_failed = matches!(item.phase, OfferPhase::Failed { .. });
                        let can_retry = matches!(item.phase, OfferPhase::Failed { retry: true, .. });
                        let label = upload_label(&name);
                        let open_label = label.clone();
                        let size_label = (item.declared_bytes > 0)
                            .then(|| bruhs_models::backup::format_bytes(item.declared_bytes));
                        let kind_icon = match item.kind {
                            FileKind::Image => "eye",
                            FileKind::Text | FileKind::Pdf | FileKind::Html => "file-text",
                            FileKind::Audio => "mic",
                            FileKind::Video | FileKind::Other => "file",
                        };
                        let tile_class = if is_failed {
                            "inline-flex min-h-control max-w-full items-center gap-1.5 rounded-grape bg-lychee-500/10 py-1 pl-2.5 pr-1 inset-ring inset-ring-lychee-400/25"
                        } else {
                            "inline-flex min-h-control max-w-full items-center gap-1.5 rounded-grape bg-longan-800/75 py-1 pl-2.5 pr-1 inset-ring inset-ring-white/10"
                        };
                        view! {
                            <div
                                data-file-offer=""
                                data-file-name=name
                                data-file-phase=phase_name
                                class=tile_class
                            >
                                <Icon
                                    name=kind_icon
                                    size=12
                                    class={if is_failed { "text-lychee-300" } else { "text-rambutan-400" }}
                                />
                                <button
                                    type="button"
                                    class="min-w-0 max-w-[180px] truncate text-left text-label-sm text-rambutan-200 transition-colors hover:text-rambutan-100 disabled:cursor-default disabled:hover:text-rambutan-200"
                                    disabled=!can_open
                                    title=label
                                    on:click=move |_| {
                                        if let Some(path) = ready.clone() {
                                            let project_path = project.get_untracked();
                                            show_open_file(open_file, panel, path, open_label.clone(), project_path, false);
                                        }
                                    }
                                >
                                    {open_label.clone()}
                                </button>
                                {size_label.map(|size| view! {
                                    <span class="shrink-0 tabular-nums text-label-sm text-rambutan-500">{size}</span>
                                })}
                                {match &item.phase {
                                    OfferPhase::Pending => view! {
                                        <span class="inline-flex shrink-0 items-center gap-1 text-label-sm text-rambutan-400">
                                            <Icon name="loader" size=12 class="animate-spin text-rambutan-500 motion-reduce:animate-none" />
                                            "Uploading"
                                        </span>
                                    }.into_any(),
                                    OfferPhase::Failed { message, .. } => view! {
                                        <span class="min-w-0 max-w-40 truncate text-label-sm text-lychee-300" title=message.clone()>{message.clone()}</span>
                                    }.into_any(),
                                    OfferPhase::Ready { .. } => ().into_any(),
                                }}
                                {can_retry.then(|| {
                                    let retry_id = retry_id.clone();
                                    view! {
                                        <button
                                            type="button"
                                            aria-label="Retry upload"
                                            class="inline-flex h-control-xs items-center rounded-seed px-1.5 text-label-sm text-rambutan-300 transition-colors hover:bg-longan-700/60 hover:text-rambutan-100 cursor-pointer"
                                            on:click=move |_| retry_offer(offers, retry_id.clone())
                                        >
                                            "Retry"
                                        </button>
                                    }
                                })}
                                <button
                                    type="button"
                                    aria-label="Remove file"
                                    class="inline-flex size-control-xs shrink-0 items-center justify-center rounded-seed text-rambutan-500 transition-colors hover:bg-longan-700/60 hover:text-rambutan-100 cursor-pointer"
                                    on:click=move |_| {
                                        let remove_id = remove_id.clone();
                                        offers.try_update(|items| items.retain(|item| item.id != remove_id));
                                    }
                                >
                                    <Icon name="x" size=16 />
                                </button>
                            </div>
                        }
                    }).collect_view()}
                </div>
            })
        }}
    }
}

#[derive(Clone)]
struct ReadyPreview {
    name: String,
    path: String,
    kind: FileKind,
    text: Option<String>,
    url: Option<String>,
    truncated: bool,
}

#[derive(Clone)]
enum Preview {
    Empty,
    Loading,
    Failed(String),
    Ready(ReadyPreview),
}

#[component]
pub fn FileViewer() -> impl IntoView {
    let OpenFileSignal(open) = expect_context::<OpenFileSignal>();
    let preview = RwSignal::new(Preview::Empty);
    let show_source = RwSignal::new(false);
    let ticket = StoredValue::new(0u32);
    let blob_url = StoredValue::new(None::<String>);
    on_cleanup(move || revoke_blob(blob_url));
    Effect::new(move |_| {
        let request = open.get();
        let mine = ticket.get_value().wrapping_add(1);
        ticket.set_value(mine);
        revoke_blob(blob_url);
        let Some(request) = request else {
            preview.set(Preview::Empty);
            return;
        };
        show_source.set(request.source);
        preview.set(Preview::Loading);
        let path = request.path;
        let project = request.project;
        let label = request.label;
        leptos::task::spawn_local(async move {
            let result = open_attachment(&path, project.as_deref()).await;
            if ticket.get_value() != mine {
                return;
            }
            match result {
                Err(error) => preview.set(Preview::Failed(error.to_string())),
                Ok(opened) => {
                    let url = opened
                        .bytes
                        .as_ref()
                        .and_then(|bytes| blob_url_for(bytes, &opened.mime));
                    if let Some(url) = url.clone() {
                        if let Some(old) = blob_url
                            .try_update_value(|slot| slot.replace(url))
                            .flatten()
                        {
                            let _ = web_sys::Url::revoke_object_url(&old);
                        }
                    }
                    let name = if label.is_empty() { opened.name } else { label };
                    preview.set(Preview::Ready(ReadyPreview {
                        name,
                        path: opened.path,
                        kind: opened.kind,
                        text: opened.text,
                        url,
                        truncated: opened.truncated,
                    }));
                }
            }
        });
    });

    let html_policy = "default-src 'none'; img-src data: blob:; style-src 'unsafe-inline' data:; font-src data:; media-src data: blob:";
    view! {
        <div class="flex h-full min-h-0 flex-col" data-file-viewer="">
            {move || match preview.get() {
                Preview::Empty => view! {
                    <p class="p-6 text-label-sm text-rambutan-500">"Select a file to read it."</p>
                }.into_any(),
                Preview::Loading => view! {
                    <p class="p-6 text-label-sm text-rambutan-500">"Opening…"</p>
                }.into_any(),
                Preview::Failed(message) => view! {
                    <p class="p-6 text-label-sm text-rambutan-400">{message}</p>
                }.into_any(),
                Preview::Ready(ready) => {
                    let name = ready.name.clone();
                    let path = ready.path.clone();
                    let kind = ready.kind;
                    let text = ready.text.clone();
                    let url = ready.url.clone();
                    let truncated = ready.truncated;
                    let source = show_source.get();
                    view! {
                        <div class="flex h-8 shrink-0 items-center gap-2 border-b border-longan-800/60 px-3">
                            <span class="min-w-0 flex-1 truncate text-label-sm text-rambutan-100" title=path.clone()>{name.clone()}</span>
                            {(kind == FileKind::Html).then(|| view! {
                                <button
                                    type="button"
                                    class="inline-flex h-6 items-center rounded-seed px-1.5 text-label-sm text-rambutan-400 hover:bg-longan-800/60 hover:text-rambutan-100 cursor-pointer"
                                    on:click=move |_| show_source.update(|value| *value = !*value)
                                >
                                    {move || if show_source.get() { "Rendered" } else { "Source" }}
                                </button>
                            })}
                        </div>
                        <div class="min-h-0 flex-1 overflow-auto">
                            {move || match kind {
                                FileKind::Image => media_image(url.clone(), name.clone()),
                                FileKind::Audio => media_audio(url.clone(), name.clone()),
                                FileKind::Video => media_video(url.clone(), name.clone()),
                                FileKind::Pdf => media_pdf(url.clone(), name.clone()),
                                FileKind::Html if !source => html_frame(text.clone().unwrap_or_default(), name.clone(), html_policy),
                                FileKind::Html | FileKind::Text => text_block(text.clone().unwrap_or_default(), truncated),
                                FileKind::Other => view! {
                                    <p class="p-4 text-label-sm text-rambutan-400">{path.clone()}</p>
                                }.into_any(),
                            }}
                        </div>
                    }.into_any()
                }
            }}
        </div>
    }
}

fn media_image(url: Option<String>, name: String) -> AnyView {
    let Some(url) = url else {
        return missing();
    };
    view! { <img src=url alt=name class="max-h-full max-w-full object-contain" /> }.into_any()
}

fn media_audio(url: Option<String>, name: String) -> AnyView {
    let Some(url) = url else {
        return missing();
    };
    view! { <audio controls src=url class="w-full" aria-label=name></audio> }.into_any()
}

fn media_video(url: Option<String>, name: String) -> AnyView {
    let Some(url) = url else {
        return missing();
    };
    view! { <video controls src=url class="max-h-full max-w-full" aria-label=name></video> }
        .into_any()
}

fn media_pdf(url: Option<String>, name: String) -> AnyView {
    let Some(url) = url else {
        return missing();
    };
    view! { <iframe src=url title=name class="h-full min-h-[320px] w-full bg-white"></iframe> }
        .into_any()
}

fn html_frame(html: String, name: String, policy: &'static str) -> AnyView {
    // The iframe csp attribute is not supported by every browser. Keep the
    // viewer's policy ahead of file markup even when the app allows media.
    let document = format!(
        "<!doctype html><html><head><meta http-equiv=\"Content-Security-Policy\" content=\"{policy}\"></head><body>{html}</body></html>"
    );
    view! {
        <iframe
            class="h-full min-h-[320px] w-full bg-white"
            sandbox=""
            title=name
            prop:srcdoc=document
        ></iframe>
    }
    .attr("csp", policy)
    .attr("referrerpolicy", "no-referrer")
    .into_any()
}

fn text_block(text: String, truncated: bool) -> AnyView {
    view! {
        <div class="p-4">
            <pre class="whitespace-pre-wrap break-words text-label-sm text-rambutan-100">{text}</pre>
            {truncated.then(|| view! {
                <p class="mt-3 text-label-sm text-rambutan-500">"This preview shows the first 256 KiB."</p>
            })}
        </div>
    }
    .into_any()
}

fn missing() -> AnyView {
    view! { <p class="p-4 text-label-sm text-rambutan-400">{UPLOAD_DID_NOT_FINISH}</p> }.into_any()
}

fn blob_url_for(bytes: &[u8], mime: &str) -> Option<String> {
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let props = web_sys::BlobPropertyBag::new();
    props.set_type(mime);
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &props).ok()?;
    web_sys::Url::create_object_url_with_blob(&blob).ok()
}

fn revoke_blob(slot: StoredValue<Option<String>>) {
    if let Some(url) = slot.try_update_value(|slot| slot.take()).flatten() {
        let _ = web_sys::Url::revoke_object_url(&url);
    }
}

fn phase_name(phase: &OfferPhase) -> &'static str {
    match phase {
        OfferPhase::Pending => "pending",
        OfferPhase::Ready { .. } => "ready",
        OfferPhase::Failed { .. } => "failed",
    }
}

fn rejected(id: &str, name: &str, declared: u64, message: &str) -> FileOffer {
    FileOffer {
        id: id.to_string(),
        name: name.to_string(),
        kind: kind_for_name(name),
        declared_bytes: declared,
        phase: OfferPhase::Failed {
            message: message.to_string(),
            retry: false,
        },
        bytes: None,
    }
}

fn committed(items: &[FileOffer], skip: Option<&str>) -> (usize, u64) {
    let mut count = 0usize;
    let mut images = 0u64;
    for item in items {
        if skip.is_some_and(|id| id == item.id) || !item.counts() {
            continue;
        }
        count += 1;
        if item.kind == FileKind::Image {
            images = images.saturating_add(item.declared_bytes);
        }
    }
    (count, images)
}

fn mark_failed(
    offers: &OfferTarget,
    id: &str,
    message: &str,
    retry: bool,
    bytes: Option<Rc<Vec<u8>>>,
) {
    let message = message.to_string();
    let id = id.to_string();
    offers.update(|items| {
        if let Some(item) = items.iter_mut().find(|item| item.id == id) {
            item.phase = OfferPhase::Failed { message, retry };
            if bytes.is_some() {
                item.bytes = bytes;
            }
        }
    });
}

async fn finish_upload(offers: OfferTarget, id: String, name: String, bytes: Vec<u8>) {
    if !offers.read(|items| items.iter().any(|item| item.id == id)) {
        return;
    }
    let actual = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let (count, images) = offers.read(|items| committed(items, Some(&id)));
    if let Err(message) = admit(count, images, &name, actual) {
        mark_failed(&offers, &id, message, false, None);
        return;
    }
    offers.update(|items| {
        if let Some(item) = items.iter_mut().find(|item| item.id == id) {
            item.declared_bytes = actual;
            item.kind = kind_for_name(&name);
        }
    });
    match upload_attachment(&name, &bytes).await {
        Ok(uploaded) => {
            offers.update(|items| {
                if let Some(item) = items.iter_mut().find(|item| item.id == id) {
                    item.name = uploaded.name;
                    item.phase = OfferPhase::Ready {
                        path: uploaded.path,
                    };
                    item.bytes = None;
                }
            });
        }
        Err(error) => {
            mark_failed(&offers, &id, &error.to_string(), true, Some(Rc::new(bytes)));
        }
    }
}
