use super::{AgentOrchestrator, PendingPermission, PermissionResolveError};
use bruhs_models::api::QuestionUpload;
use bruhs_models::logic::attachments::{
    admit, kind_for_name, FileKind, UploadedFile, UPLOAD_DID_NOT_FINISH,
};
use bruhs_models::logic::user_question::questions;
use uuid::Uuid;

type ReplyError = PermissionResolveError;

/// Until submitted, the waiting request owns the only lease on this file.
pub(super) struct QuestionFile {
    id: Uuid,
    question: usize,
    device: Option<Uuid>,
    size: u64,
    file: Option<UploadedFile>,
}

impl Drop for QuestionFile {
    fn drop(&mut self) {
        if let Some(file) = &self.file {
            if let Err(error) = std::fs::remove_file(&file.path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(%error, "Could not remove a staged answer file");
                }
            }
        }
    }
}

impl<R> PendingPermission<R> {
    fn can_upload(&self, question: usize) -> Result<(), ReplyError> {
        if questions(&self.input)
            .and_then(|items| items.get(question).map(|item| item.allow_custom))
            == Some(true)
        {
            Ok(())
        } else {
            Err(ReplyError::Question("This question does not accept files."))
        }
    }

    fn admit_file(&self, name: &str, size: u64) -> Result<(), ReplyError> {
        let image_bytes = self
            .question_files
            .iter()
            .filter(|upload| {
                upload
                    .file
                    .as_ref()
                    .is_some_and(|file| kind_for_name(&file.name) == FileKind::Image)
            })
            .map(|upload| upload.size)
            .sum();
        admit(self.question_files.len(), image_bytes, name, size)
            .map(|_| ())
            .map_err(ReplyError::Question)
    }

    fn retried_file(
        &self,
        upload: &QuestionUpload,
        device: Option<Uuid>,
    ) -> Result<Option<UploadedFile>, ReplyError> {
        let Some(existing) = self
            .question_files
            .iter()
            .find(|item| item.id == upload.upload_id)
        else {
            return Ok(None);
        };
        if existing.question != upload.question || existing.device != device {
            return Err(ReplyError::Question(
                "This file is unavailable for this answer.",
            ));
        }
        Ok(existing.file.clone())
    }

    pub(super) fn files_for(
        &self,
        question: usize,
        ids: &[Uuid],
        device: Option<Uuid>,
    ) -> Result<Vec<UploadedFile>, &'static str> {
        ids.iter()
            .map(|id| {
                self.question_files
                    .iter()
                    .find(|upload| {
                        upload.id == *id && upload.question == question && upload.device == device
                    })
                    .and_then(|upload| upload.file.clone())
                    .ok_or("A file in this answer is no longer available.")
            })
            .collect()
    }

    pub(super) fn selected_files(&self, selected: &[Uuid]) -> Vec<UploadedFile> {
        selected
            .iter()
            .filter_map(|id| self.question_files.iter().find(|upload| upload.id == *id))
            .filter_map(|upload| upload.file.clone())
            .collect()
    }

    pub(super) fn preserve_files(&mut self, selected: &[Uuid]) {
        for upload in &mut self.question_files {
            if selected.contains(&upload.id) {
                upload.file.take();
            }
        }
    }
}

impl AgentOrchestrator {
    /// Stage one file against a specific active question. Retried uploads reuse
    /// the same identifier. The second check also covers concurrent turn exit.
    pub(crate) async fn upload_question_file(
        &self,
        upload: QuestionUpload,
        bytes: impl AsRef<[u8]> + Send + 'static,
        device: Option<Uuid>,
        desktop_owned: bool,
    ) -> Result<UploadedFile, ReplyError> {
        let size = u64::try_from(bytes.as_ref().len()).unwrap_or(u64::MAX);
        let owner = {
            let pending = self.permissions.read().await;
            let request = pending
                .get(&upload.request_id)
                .filter(|request| request.owner.thread_id == upload.thread_id && request.can_answer)
                .ok_or(ReplyError::UnknownRequest)?;
            request.can_upload(upload.question)?;
            if let Some(file) = request.retried_file(&upload, device)? {
                return Ok(file);
            }
            request.admit_file(&upload.name, size)?;
            request.owner.clone()
        };

        let name = upload.name.clone();
        let id = upload.upload_id;
        let question = upload.question;
        let thread = upload.thread_id.clone();
        let file = tokio::task::spawn_blocking(move || {
            let file = if desktop_owned {
                crate::uploads::write_desktop_question_upload(&name, bytes.as_ref())
            } else {
                crate::uploads::write_web_question_upload(&thread, &name, bytes.as_ref())
            }?;
            Ok::<_, &'static str>(QuestionFile {
                id,
                question,
                device,
                size,
                file: Some(file),
            })
        })
        .await
        .map_err(|_| ReplyError::Question(UPLOAD_DID_NOT_FINISH))?
        .map_err(ReplyError::Question)?;
        let receipt = file
            .file
            .as_ref()
            .cloned()
            .ok_or(ReplyError::Question(UPLOAD_DID_NOT_FINISH))?;
        let mut pending = self.permissions.write().await;
        let request = pending
            .get_mut(&upload.request_id)
            .filter(|request| request.owner == owner && request.can_answer)
            .ok_or(ReplyError::UnknownRequest)?;
        request.can_upload(upload.question)?;
        if let Some(file) = request.retried_file(&upload, device)? {
            return Ok(file);
        }
        request.admit_file(&upload.name, size)?;
        request.question_files.push(file);
        Ok(receipt)
    }

    pub(crate) async fn remove_question_file(
        &self,
        upload: &QuestionUpload,
        device: Option<Uuid>,
    ) -> Result<(), ReplyError> {
        let mut pending = self.permissions.write().await;
        let request = pending
            .get_mut(&upload.request_id)
            .filter(|request| request.owner.thread_id == upload.thread_id && request.can_answer)
            .ok_or(ReplyError::UnknownRequest)?;
        request.question_files.retain(|file| {
            file.id != upload.upload_id || file.question != upload.question || file.device != device
        });
        Ok(())
    }
}
