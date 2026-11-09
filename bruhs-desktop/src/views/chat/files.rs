//! Composer file offers. A picked, dropped, or pasted file is written under
//! the attachment directory as soon as it is admitted. Send waits until every
//! offer is ready.

use super::{ChatView, ChatViewEvent};
use bruhs_models::logic::attachments::{
    admit, kind_for_name, send_blocked, upload_label, FileKind, UploadPhase, UploadedFile,
    COULD_NOT_READ, TOO_MANY_FILES, UPLOAD_DID_NOT_FINISH,
};
use bruhs_models::logic::commands::{append_goal, ThreadGoal};
use bruhs_models::logic::composer_text::{agent_prompt, visible_prompt, TextAttachment};
use gpui::{AppContext as _, Context, PathPromptOptions, Window};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub(super) struct DesktopFileOffer {
    pub id: String,
    pub name: String,
    pub kind: FileKind,
    pub declared_bytes: u64,
    phase: DesktopOfferPhase,
    bytes: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
enum DesktopOfferPhase {
    Pending,
    Ready { file: UploadedFile },
    Failed { message: String, retry: bool },
}

impl DesktopFileOffer {
    pub(super) fn upload_phase(&self) -> UploadPhase {
        match self.phase {
            DesktopOfferPhase::Pending => UploadPhase::Pending,
            DesktopOfferPhase::Ready { .. } => UploadPhase::Ready,
            DesktopOfferPhase::Failed { .. } => UploadPhase::Failed,
        }
    }

    fn counts(&self) -> bool {
        !matches!(self.phase, DesktopOfferPhase::Failed { retry: false, .. })
    }

    pub(super) fn ready_file(&self) -> Option<UploadedFile> {
        match &self.phase {
            DesktopOfferPhase::Ready { file } => Some(file.clone()),
            _ => None,
        }
    }

    pub(super) fn status_text(&self) -> Option<&str> {
        match &self.phase {
            DesktopOfferPhase::Pending => Some("Uploading"),
            DesktopOfferPhase::Failed { message, .. } => Some(message.as_str()),
            DesktopOfferPhase::Ready { .. } => None,
        }
    }

    pub(super) fn status_is_error(&self) -> bool {
        matches!(self.phase, DesktopOfferPhase::Failed { .. })
    }

    pub(super) fn can_retry(&self) -> bool {
        matches!(self.phase, DesktopOfferPhase::Failed { retry: true, .. })
    }

    pub(super) fn can_open(&self) -> bool {
        matches!(self.phase, DesktopOfferPhase::Ready { .. })
    }
}

pub(super) fn offers_block_send(offers: &[DesktopFileOffer]) -> bool {
    let phases: Vec<UploadPhase> = offers.iter().map(DesktopFileOffer::upload_phase).collect();
    send_blocked(&phases)
}

/// The provider prompt. Paste files are named first. Uploaded files follow
/// the goal, and an empty upload list leaves the prompt unchanged.
pub(super) fn turn_prompt(
    visible: &str,
    paste_files: &[(String, String)],
    uploads: &[UploadedFile],
    goal: Option<&ThreadGoal>,
) -> String {
    bruhs_models::logic::attachments::append_uploaded_files(
        &append_goal(&agent_prompt(visible, paste_files), goal),
        uploads,
    )
}

/// Visible transcript text. File names are listed as attachments and are not
/// sent as pasted text.
pub(super) fn visible_with_files(
    expanded: &str,
    pastes: &[TextAttachment],
    uploads: &[UploadedFile],
) -> String {
    let mut named = pastes.to_vec();
    for file in uploads {
        named.push(TextAttachment {
            name: file.name.clone(),
            text: String::new(),
        });
    }
    if named.is_empty() {
        expanded.trim().to_string()
    } else {
        visible_prompt(expanded, &named)
    }
}

fn committed(offers: &[DesktopFileOffer], skip: Option<&str>) -> (usize, u64) {
    let mut count = 0usize;
    let mut images = 0u64;
    for offer in offers {
        if skip.is_some_and(|id| id == offer.id) || !offer.counts() {
            continue;
        }
        count += 1;
        if offer.kind == FileKind::Image {
            images = images.saturating_add(offer.declared_bytes);
        }
    }
    (count, images)
}

impl ChatView {
    pub(super) fn uploads_block_send(&self) -> bool {
        !self.is_streaming && offers_block_send(&self.file_offers)
    }

    pub(super) fn has_ready_upload(&self) -> bool {
        self.file_offers
            .iter()
            .any(|offer| offer.ready_file().is_some())
    }

    pub(super) fn attach_files(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach files".into()),
        });
        let chat = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let Ok(Ok(Some(paths))) = picker.await else {
                    return;
                };
                let _ = chat.update(cx, |this, cx| {
                    this.enqueue_paths(&paths, cx);
                });
            })
            .detach();
    }

    pub(super) fn enqueue_paths(&mut self, paths: &[PathBuf], cx: &mut Context<'_, Self>) {
        for path in paths {
            self.enqueue_path_bytes(path.clone(), None, cx);
        }
    }

    pub(super) fn enqueue_named_bytes(
        &mut self,
        name: String,
        bytes: Vec<u8>,
        cx: &mut Context<'_, Self>,
    ) {
        let declared = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if let Some(message) = self.push_offer(&name, declared, Some(bytes), cx) {
            cx.emit(ChatViewEvent::Notice(message.to_string()));
        }
    }

    pub(super) fn remove_file_offer(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        self.file_offers.retain(|offer| offer.id != id);
        cx.notify();
    }

    pub(super) fn retry_file_offer(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        let Some(offer) = self.file_offers.iter_mut().find(|offer| offer.id == id) else {
            return;
        };
        let Some(bytes) = offer.bytes.clone() else {
            return;
        };
        let name = offer.name.clone();
        offer.phase = DesktopOfferPhase::Pending;
        cx.notify();
        self.write_offer(id.to_string(), name, bytes, cx);
    }

    pub(super) fn open_ready_file(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        let Some(offer) = self.file_offers.iter().find(|offer| offer.id == id) else {
            return;
        };
        let Some(file) = offer.ready_file() else {
            return;
        };
        let label = offer.name.clone();
        let project = self
            .project_scoped
            .then(|| self.project_path.clone())
            .filter(|path| !path.is_empty());
        cx.emit(ChatViewEvent::OpenFile {
            path: file.path,
            label,
            project,
            source: false,
        });
    }

    fn enqueue_path_bytes(
        &mut self,
        path: PathBuf,
        inline: Option<Vec<u8>>,
        cx: &mut Context<'_, Self>,
    ) {
        let name = path.file_name().map_or_else(
            || "file".to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let meta = std::fs::metadata(&path).ok();
        let declared = meta.as_ref().map_or(0, |meta| meta.len());
        let is_file = meta.as_ref().is_some_and(|meta| meta.is_file());
        if meta.is_some() && !is_file {
            self.push_rejected(&name, declared, "That path is not a file.", cx);
            return;
        }
        let inline_present = inline.is_some();
        if let Some(message) = self.push_offer(&name, declared, inline, cx) {
            cx.emit(ChatViewEvent::Notice(message.to_string()));
            return;
        }
        if inline_present {
            return;
        }
        let Some(id) = self.file_offers.last().map(|offer| offer.id.clone()) else {
            return;
        };
        if !matches!(
            self.file_offers.last().map(|offer| offer.upload_phase()),
            Some(UploadPhase::Pending)
        ) {
            return;
        }
        let read = cx.background_spawn(async move { std::fs::read(&path) });
        let chat = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let read = read.await;
            let _ = chat.update(cx, |this, cx| match read {
                Err(_) => this.fail_offer(&id, COULD_NOT_READ, false, None, cx),
                Ok(bytes) => this.accept_bytes(id, name, bytes, cx),
            });
        })
        .detach();
    }

    /// `Some` when the message is already full and nothing was added.
    /// A rejection is added as a failed offer and returns `None`.
    fn push_offer(
        &mut self,
        name: &str,
        declared: u64,
        bytes: Option<Vec<u8>>,
        cx: &mut Context<'_, Self>,
    ) -> Option<&'static str> {
        if self.file_offers.len() >= bruhs_models::logic::attachments::MAX_FILES_PER_MESSAGE {
            return Some(TOO_MANY_FILES);
        }
        let (count, image_bytes) = committed(&self.file_offers, None);
        let id = uuid::Uuid::new_v4().to_string();
        match admit(count, image_bytes, name, declared) {
            Err(message) => {
                self.file_offers
                    .push(rejected(&id, name, declared, message));
                cx.notify();
                None
            }
            Ok(kind) => {
                let offer_name = upload_label(name);
                let offer_name = if offer_name.is_empty() {
                    "file".to_string()
                } else {
                    offer_name
                };
                self.file_offers.push(DesktopFileOffer {
                    id: id.clone(),
                    name: offer_name.clone(),
                    kind,
                    declared_bytes: declared,
                    phase: DesktopOfferPhase::Pending,
                    bytes: None,
                });
                cx.notify();
                if let Some(bytes) = bytes {
                    self.write_offer(id, offer_name, bytes, cx);
                }
                None
            }
        }
    }

    fn push_rejected(
        &mut self,
        name: &str,
        declared: u64,
        message: &str,
        cx: &mut Context<'_, Self>,
    ) {
        let id = uuid::Uuid::new_v4().to_string();
        self.file_offers
            .push(rejected(&id, name, declared, message));
        cx.notify();
    }

    fn accept_bytes(
        &mut self,
        id: String,
        name: String,
        bytes: Vec<u8>,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.file_offers.iter().any(|offer| offer.id == id) {
            return;
        }
        let actual = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let (count, images) = committed(&self.file_offers, Some(&id));
        if let Err(message) = admit(count, images, &name, actual) {
            self.fail_offer(&id, message, false, None, cx);
            return;
        }
        if let Some(offer) = self.file_offers.iter_mut().find(|offer| offer.id == id) {
            offer.declared_bytes = actual;
            offer.kind = kind_for_name(&name);
        }
        self.write_offer(id, name, bytes, cx);
    }

    fn write_offer(
        &mut self,
        id: String,
        name: String,
        bytes: Vec<u8>,
        cx: &mut Context<'_, Self>,
    ) {
        let written = cx.background_spawn(async move {
            match bruhs_core::write_upload(&name, &bytes) {
                Ok(file) => Ok((file, bytes)),
                Err(message) => Err((message, bytes)),
            }
        });
        let chat = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let written = written.await;
            let _ = chat.update(cx, |this, cx| match written {
                Ok((file, bytes)) => this.ready_offer(id, file, bytes, cx),
                Err((message, bytes)) => {
                    let retry = message == UPLOAD_DID_NOT_FINISH;
                    let kept = retry.then_some(bytes);
                    this.fail_offer(&id, message, retry, kept, cx);
                }
            });
        })
        .detach();
    }

    fn ready_offer(
        &mut self,
        id: String,
        file: UploadedFile,
        bytes: Vec<u8>,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(offer) = self.file_offers.iter_mut().find(|offer| offer.id == id) else {
            return;
        };
        offer.phase = DesktopOfferPhase::Ready { file };
        offer.bytes = Some(bytes);
        cx.notify();
    }

    fn fail_offer(
        &mut self,
        id: &str,
        message: &str,
        retry: bool,
        bytes: Option<Vec<u8>>,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(offer) = self.file_offers.iter_mut().find(|offer| offer.id == id) else {
            return;
        };
        if bytes.is_some() {
            offer.bytes = bytes;
        }
        offer.phase = DesktopOfferPhase::Failed {
            message: message.to_string(),
            retry,
        };
        cx.notify();
    }
}

fn rejected(id: &str, name: &str, declared: u64, message: &str) -> DesktopFileOffer {
    DesktopFileOffer {
        id: id.to_string(),
        name: upload_label(name),
        kind: kind_for_name(name),
        declared_bytes: declared,
        phase: DesktopOfferPhase::Failed {
            message: message.to_string(),
            retry: false,
        },
        bytes: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::logic::attachments::HEIC_NOT_CONVERTED;

    #[test]
    fn uploads_are_named_after_the_goal_and_absent_uploads_leave_it() {
        let goal = ThreadGoal {
            text: "Ship the viewer".into(),
            paused: false,
        };
        let plain = turn_prompt("hello", &[], &[], Some(&goal));
        assert!(plain.contains("hello"));
        assert!(plain.contains("Ship the viewer"));
        assert!(!plain.contains("Read these attached files"));

        let uploads = vec![UploadedFile {
            name: "note.txt".into(),
            path: "/tmp/note.txt".into(),
        }];
        let with_file = turn_prompt("hello", &[], &uploads, Some(&goal));
        assert!(with_file.contains("Read these attached files\nnote.txt /tmp/note.txt"));
        assert!(with_file.contains(&plain));
        let visible = visible_with_files("  hello  ", &[], &uploads);
        assert_eq!(visible, "hello\n\nAttached: note.txt");
        assert_eq!(visible_with_files("  hello  ", &[], &[]), "hello");
    }

    #[test]
    fn a_failed_offer_blocks_send_until_it_is_removed() {
        let failed = DesktopFileOffer {
            id: "1".into(),
            name: "photo.HEIC".into(),
            kind: FileKind::Image,
            declared_bytes: 4,
            phase: DesktopOfferPhase::Failed {
                message: HEIC_NOT_CONVERTED.into(),
                retry: false,
            },
            bytes: None,
        };
        assert!(offers_block_send(&[failed]));
        assert!(!offers_block_send(&[]));
    }
}
