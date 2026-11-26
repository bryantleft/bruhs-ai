use super::*;
use bruhs_models::api::QuestionUpload;
use bruhs_models::logic::attachments::{
    COULD_NOT_READ, MAX_FILES_PER_MESSAGE, MAX_FILE_BYTES, TOO_MANY_FILES,
};
use std::io::Read as _;

#[derive(Clone)]
enum Source {
    Path(PathBuf),
    Clipboard(Arc<[u8]>),
}

pub(super) struct Offer {
    upload: QuestionUpload,
    source: Source,
    result: Option<Result<(), String>>,
}

impl Offer {
    pub(super) fn question(&self) -> usize {
        self.upload.question
    }

    pub(super) fn ready(&self) -> bool {
        matches!(self.result, Some(Ok(())))
    }
}

impl QuestionPrompt {
    pub(super) fn choose_files(&mut self, cx: &mut Context<'_, Self>) {
        self.auto_advance = None;
        let pick = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach files to the answer".into()),
        });
        let question = self.position;
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = pick.await {
                let _ = this.update(cx, |this, cx| this.add_paths(question, &paths, cx));
            }
        })
        .detach();
    }

    fn add_paths(&mut self, question: usize, paths: &[PathBuf], cx: &mut Context<'_, Self>) {
        for path in paths {
            let name = path.file_name().map_or_else(
                || "file".to_string(),
                |name| name.to_string_lossy().into_owned(),
            );
            self.add_file(question, name, Source::Path(path.clone()), cx);
        }
    }

    pub(super) fn paste_files(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        if self.busy
            || !self.fields[self.position]
                .as_ref()
                .is_some_and(|field| field.read(cx).focus_handle(cx).is_focused(window))
        {
            return false;
        }
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        let question = self.position;
        let mut handled = false;
        for entry in item.entries() {
            match entry {
                ClipboardEntry::ExternalPaths(paths) => {
                    self.add_paths(question, paths.paths(), cx);
                    handled = true;
                }
                ClipboardEntry::Image(image) => {
                    let name = super::super::streaming::pasted_image_name(image.format).to_string();
                    self.add_file(
                        question,
                        name,
                        Source::Clipboard(Arc::from(image.bytes.as_slice())),
                        cx,
                    );
                    handled = true;
                }
                ClipboardEntry::String(_) => {}
            }
        }
        handled
    }

    fn add_file(
        &mut self,
        question: usize,
        name: String,
        source: Source,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy
            || !self
                .questions
                .get(question)
                .is_some_and(|question| question.allow_custom)
        {
            return;
        }
        if self.uploads.len() >= MAX_FILES_PER_MESSAGE {
            self.error = Some(TOO_MANY_FILES.into());
            cx.notify();
            return;
        }
        let Some(thread_id) = self.chat.upgrade().and_then(|chat| {
            let chat = chat.read(cx);
            ThreadId::try_from(chat.thread_id.as_deref()?).ok()
        }) else {
            return;
        };
        let upload = QuestionUpload {
            thread_id,
            request_id: self.request.id.clone(),
            question,
            upload_id: uuid::Uuid::new_v4(),
            name,
        };
        self.uploads.push(Offer {
            upload: upload.clone(),
            source,
            result: None,
        });
        self.upload_file(upload.upload_id, cx);
    }

    fn upload_file(&mut self, id: uuid::Uuid, cx: &mut Context<'_, Self>) {
        let Some(offer) = self
            .uploads
            .iter_mut()
            .find(|offer| offer.upload.upload_id == id)
        else {
            return;
        };
        let Some(state) = cx.try_global::<DesktopState>() else {
            return;
        };
        let app = state.app.clone();
        let (upload, source) = (offer.upload.clone(), offer.source.clone());
        let question = upload.question;
        offer.result = None;
        self.error = None;
        let task = state.runtime.spawn(async move {
            let bytes = match source {
                Source::Clipboard(bytes) => bytes,
                Source::Path(path) => tokio::task::spawn_blocking(move || read_file(&path))
                    .await
                    .map_err(|_| COULD_NOT_READ.to_string())??,
            };
            app.upload_desktop_question_file(upload, bytes)
                .await
                .map(|_| ())
                .map_err(|err| err.to_string())
        });
        cx.spawn(async move |this, cx| {
            let result = task
                .await
                .unwrap_or_else(|_| Err("This upload did not finish.".into()));
            let _ = this.update(cx, |this, cx| {
                if let Some(offer) = this
                    .uploads
                    .iter_mut()
                    .find(|offer| offer.upload.upload_id == id)
                {
                    if result.is_ok() {
                        this.answers[question].attachments.push(id);
                    }
                    offer.result = Some(result);
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn remove_file(&mut self, id: uuid::Uuid, cx: &mut Context<'_, Self>) {
        let Some(offer) = self
            .uploads
            .iter_mut()
            .find(|offer| offer.upload.upload_id == id)
        else {
            return;
        };
        let Some(state) = cx.try_global::<DesktopState>() else {
            return;
        };
        let app = state.app.clone();
        let upload = offer.upload.clone();
        let question = upload.question;
        let previous = offer.result.take();
        let task = state
            .runtime
            .spawn(async move { app.remove_question_file(&upload).await });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if matches!(
                    result,
                    Ok(Ok(()) | Err(PermissionResolveError::UnknownRequest))
                ) {
                    this.uploads.retain(|offer| offer.upload.upload_id != id);
                    this.answers[question]
                        .attachments
                        .retain(|item| *item != id);
                } else if let Some(offer) = this
                    .uploads
                    .iter_mut()
                    .find(|offer| offer.upload.upload_id == id)
                {
                    offer.result = previous;
                    this.error = Some("Could not remove the attachment.".into());
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    #[cfg(test)]
    pub(super) fn record_fixture_upload(&mut self, question: usize) {
        let upload_id = uuid::Uuid::new_v4();
        self.uploads.push(Offer {
            upload: QuestionUpload {
                thread_id: ThreadId::new_v4(),
                request_id: self.request.id.clone(),
                question,
                upload_id,
                name: "release-notes.txt".into(),
            },
            source: Source::Clipboard(Arc::from(&b"fixture"[..])),
            result: Some(Ok(())),
        });
        self.answers[question].attachments.push(upload_id);
    }

    pub(super) fn render_files(&self, cx: &Context<'_, Self>) -> impl IntoElement {
        let position = self.position;
        let tiles = self.uploads.iter().filter(|offer| offer.upload.question == position).enumerate().map(|(index, offer)| {
            let id = offer.upload.upload_id;
            let button = |key, label, disabled, action: fn(&mut Self, uuid::Uuid, &mut Context<'_, Self>)| {
                web_button((key, index), WebButtonVariant::Ghost, WebButtonSize::Small, ButtonState::disabled_if(disabled))
                    .aria_label(label)
                    .child(label)
                    .map(|button| on_press_view(button, cx, move |this, _, cx| {
                        if !disabled { action(this, id, cx); }
                    }))
            };
            div()
                .debug_selector(move || format!("agent-question-file-{index}"))
                .flex()
                .items_center()
                .gap_1()
                .rounded(px(6.0))
                .border_1()
                .border_color(theme::longan_800()).px_2().min_w_0().max_w_full()
                .child(div().truncate().child(offer.upload.name.clone()))
                .when(offer.result.is_none(), |tile| tile.child("Working"))
                .when_some(offer.result.as_ref().and_then(|value| value.as_ref().err()), |tile, error| {
                    tile.child(div().text_color(theme::lychee_300()).truncate().child(error.clone()))
                        .child(button("question-file-retry", "Retry", self.busy, Self::upload_file))
                })
                .child(button("question-file-remove", "Remove", self.busy || offer.result.is_none(), Self::remove_file))
        });
        div()
            .flex()
            .items_start()
            .gap_2()
            .min_w_0()
            .child(self.action(
                "agent-question-attach",
                "Attach files",
                WebButtonVariant::Ghost,
                self.busy,
                cx,
                Self::choose_files,
            ))
            .child(
                div()
                    .id("question-files")
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .min_w_0()
                    .max_h(px(84.0))
                    .overflow_y_scroll()
                    .children(tiles),
            )
    }
}

fn read_file(path: &std::path::Path) -> Result<Arc<[u8]>, String> {
    let metadata = std::fs::metadata(path).map_err(|_| COULD_NOT_READ.to_string())?;
    if !metadata.is_file() {
        return Err("That path is not a file.".into());
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    bruhs_models::logic::attachments::admit(0, 0, &name, metadata.len()).map_err(str::to_owned)?;
    let file = std::fs::File::open(path).map_err(|_| COULD_NOT_READ.to_string())?;
    if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        return Err("That path is not a file.".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| COULD_NOT_READ.to_string())?;
    Ok(Arc::from(bytes))
}
