//! The mobile projection borrows the native conversation, never the companion store.
use super::*;
use bruhs_core::mobile::{MobileError, MobileQuestionFile, MobileResult};
use bruhs_models::mobile::*;
use sha2::{Digest, Sha256};
use std::io::Write;

struct Review {
    digest: Sha256,
    prefix: Vec<u8>,
    bytes: usize,
}
impl Write for Review {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.digest.update(bytes);
        self.bytes = self.bytes.saturating_add(bytes.len());
        let remaining = TEXT_BYTES.saturating_sub(self.prefix.len());
        self.prefix
            .extend_from_slice(&bytes[..remaining.min(bytes.len())]);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl ChatView {
    pub(crate) fn mobile_needs_answer(&self) -> bool {
        self.pending_permission
            .as_ref()
            .is_some_and(|request| request.can_answer)
    }

    pub(crate) fn mobile_state(&self) -> MobileThreadState {
        if self.pending_secret.is_some() || self.mobile_needs_answer() {
            MobileThreadState::LaptopAction
        } else if self.pending_permission.is_some() {
            MobileThreadState::NeedsApproval
        } else if self.is_streaming || self.placing {
            MobileThreadState::Running
        } else {
            MobileThreadState::Idle
        }
    }
    fn mobile_permission(&self) -> Option<MobilePermission> {
        let request = self.pending_permission.as_ref()?;
        if request.can_answer || request.id.is_empty() || request.id.len() > 128 {
            return None;
        }
        let review = review_request(request)?;
        let full = review.bytes;
        let hash = format!("{:x}", review.digest.finalize());
        let prefix = String::from_utf8_lossy(&review.prefix);
        let input = bruhs_models::secrets::review_text(&prefix);
        let tool = bruhs_models::secrets::review_text(&request.tool);
        let can_allow = full <= TEXT_BYTES && input.len() <= TEXT_BYTES && tool.len() <= 128;
        let (input, _) = text_slice(&input, 0, TEXT_BYTES).ok()?;
        Some(MobilePermission {
            request_id: request.id.clone(),
            turn_generation: self.turn_generation,
            tool: mobile_label(&request.tool, 128),
            input: input.to_owned(),
            review_hash: hash,
            can_allow,
        })
    }

    fn mobile_question(&self) -> Option<MobileQuestion> {
        let request = self.pending_permission.as_ref()?;
        if self.pending_secret.is_some()
            || !request.can_answer
            || request.id.is_empty()
            || request.id.len() > 128
        {
            return None;
        }
        let mut questions = bruhs_models::logic::user_question::questions(&request.input)?;
        for question in &mut questions {
            question.question = mobile_question_text(&question.question);
            question.header = mobile_question_text(&question.header);
            for option in &mut question.options {
                option.label = mobile_question_text(&option.label);
                option.description = mobile_question_text(&option.description);
                option.preview = option.preview.as_deref().map(mobile_question_text);
            }
        }
        let response = self.current_question_response();
        let mut question = MobileQuestion {
            request_id: request.id.clone(),
            turn_generation: self.turn_generation,
            review_hash: format!("{:x}", review_request(request)?.digest.finalize()),
            questions,
            responding: response.is_some_and(|response| response.responding()),
            error: response
                .and_then(|response| response.error.as_ref())
                .map(|_| "The answer could not be delivered. Try again.".to_owned()),
            can_attach: true,
            markdown: Vec::new(),
        };
        // Keep enough space for the thread metadata and history. A partial
        // question could omit an option or change what the device is reviewing.
        let budget = RESPONSE_BYTES - TEXT_BYTES;
        if serde_json::to_vec(&question).ok()?.len() > budget {
            return None;
        }
        question.markdown = question
            .questions
            .iter()
            .map(|item| MobileQuestionMarkdown {
                question: mobile_question_markdown(&item.question),
                previews: item
                    .options
                    .iter()
                    .map(|option| option.preview.as_deref().and_then(mobile_question_markdown))
                    .collect(),
            })
            .collect();
        if question
            .markdown
            .iter()
            .all(|item| item.question.is_none() && item.previews.iter().all(Option::is_none))
            || serde_json::to_vec(&question).ok()?.len() > budget
        {
            question.markdown.clear();
        }
        Some(question)
    }

    fn mobile_question_review(
        &self,
        turn_generation: u64,
        request_id: &str,
        review_hash: &str,
    ) -> MobileResult<MobileQuestion> {
        let question = self.mobile_question().ok_or_else(changed)?;
        if question.turn_generation != turn_generation
            || question.request_id != request_id
            || question.review_hash != review_hash
        {
            return Err(changed());
        }
        if question.responding {
            return Err(MobileError::Conflict(
                "An answer is already being sent.".into(),
            ));
        }
        Ok(question)
    }

    pub(crate) fn check_mobile_question_file(&self, file: &MobileQuestionFile) -> MobileResult<()> {
        let question = self.mobile_question_review(
            file.turn_generation,
            &file.upload.request_id,
            &file.review_hash,
        )?;
        if question
            .questions
            .get(file.upload.question)
            .is_none_or(|question| !question.allow_custom)
        {
            return Err(MobileError::Invalid(
                "This question does not accept files.".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn mobile_question_link(
        &self,
        read: &MobileQuestionLink,
    ) -> MobileResult<MobileProjectLink> {
        let review =
            self.mobile_question_review(read.turn_generation, &read.request_id, &read.review_hash)?;
        if !self.project_scoped {
            return Err(MobileError::Forbidden);
        }
        review
            .questions
            .iter()
            .flat_map(|item| {
                std::iter::once(item.question.as_str()).chain(
                    item.options
                        .iter()
                        .filter_map(|option| option.preview.as_deref()),
                )
            })
            .find_map(|markdown| mobile_question_project_link(markdown, &read.link))
            .ok_or_else(changed)
    }

    pub(crate) fn mobile_snapshot(
        &self,
        thread: MobileThread,
        before: Option<usize>,
    ) -> MobileResult<MobileThreadSnapshot> {
        let end = before.unwrap_or(self.messages.len());
        if end > self.messages.len() {
            return Err(changed());
        }
        let queued = self
            .thread_id
            .as_deref()
            .map_or(0, |id| self.prompt_queue.for_thread(id).len());
        let permission = self.mobile_permission();
        let question = self.mobile_question();
        let laptop_action_required = self.pending_secret.is_some()
            || (self.pending_permission.is_some() && permission.is_none());
        let mut snapshot = MobileThreadSnapshot {
            thread,
            messages: Vec::new(),
            next_before: None,
            permission,
            question,
            queued_prompts: queued,
            laptop_action_required,
        };
        let mut used = serde_json::to_vec(&MobileReadReply::Thread(snapshot.clone()))
            .map_err(|_| changed())?
            .len();
        for index in (0..end).rev().take(PAGE_SIZE) {
            let message = &self.messages[index];
            // Six JSON bytes per source byte is the escaping worst case. Leave row metadata room.
            let remaining = RESPONSE_BYTES.saturating_sub(used.saturating_add(512));
            if remaining < 6 && !snapshot.messages.is_empty() {
                break;
            }
            let budget = TEXT_BYTES.min(remaining / 6);
            if budget == 0 {
                break;
            }
            let (offset, text) = text_tail(&message.content, budget);
            let row = MobileMessage {
                index,
                role: message.role.as_str().to_owned(),
                text: text.to_owned(),
                byte_offset: offset,
                full_bytes: message.content.len(),
                streaming: message.state.is_streaming(),
                reference: message_reference(message, index),
            };
            used += serde_json::to_vec(&row).map_err(|_| changed())?.len() + 1;
            if used > RESPONSE_BYTES {
                break;
            }
            snapshot.messages.push(row);
        }
        snapshot.messages.reverse();
        snapshot.next_before = snapshot
            .messages
            .first()
            .and_then(|m| (m.index > 0).then_some(m.index));
        if end > 0 && snapshot.messages.is_empty() {
            return Err(MobileError::Invalid(
                "This approval leaves no room for chat history. Review it on the laptop.".into(),
            ));
        }
        Ok(snapshot)
    }
    pub(crate) fn mobile_message(
        &self,
        index: usize,
        reference: &str,
        offset: usize,
        revision: Option<&str>,
    ) -> MobileResult<MobileTextPage> {
        let message = self
            .messages
            .get(index)
            .filter(|m| message_reference(m, index) == reference)
            .ok_or_else(changed)?;
        let actual = format!("{:x}", Sha256::digest(message.content.as_bytes()));
        if (offset > 0 && revision.is_none()) || revision.is_some_and(|r| r != actual) {
            return Err(changed());
        }
        let (text, next_offset) =
            text_slice(&message.content, offset, TEXT_BYTES).map_err(|_| changed())?;
        Ok(MobileTextPage {
            text: text.to_owned(),
            offset,
            next_offset,
            revision: actual,
            source_truncated: false,
        })
    }
    pub(crate) fn mobile_action(
        &mut self,
        action: &MobileAction,
        device: uuid::Uuid,
        cx: &mut Context<'_, Self>,
    ) -> MobileResult<MobileCommandOutcome> {
        match action {
            MobileAction::OpenLinkedDraft { .. } | MobileAction::ContinueLinkedDraft { .. } => Err(
                MobileError::Invalid("Open draft links through the workspace.".into()),
            ),
            MobileAction::Prompt { text, .. } => {
                let thread = self.thread_id.as_deref().ok_or_else(changed)?;
                let busy = self.is_streaming
                    || self.placing
                    || self.queue_held
                    || self.prompt_queue.has_for(thread);
                let started = self.accept_thread_message(text, cx);
                if !busy && !started {
                    return Err(MobileError::Invalid(
                        "The desktop could not start this prompt. Check the thread on the laptop."
                            .into(),
                    ));
                }
                Ok(MobileCommandOutcome::Accepted { queued: busy })
            }
            MobileAction::Permission {
                turn_generation,
                request_id,
                review_hash,
                decision,
                ..
            } => {
                let review = self.mobile_permission().ok_or_else(changed)?;
                if review.turn_generation != *turn_generation
                    || review.request_id != *request_id
                    || review.review_hash != *review_hash
                {
                    return Err(changed());
                }
                if *decision == MobileDecision::Allow && !review.can_allow {
                    return Err(MobileError::Forbidden);
                }
                self.respond_to_permission(*decision == MobileDecision::Allow, cx);
                Ok(MobileCommandOutcome::Accepted { queued: false })
            }
            MobileAction::AnswerQuestion {
                turn_generation,
                request_id,
                review_hash,
                reply,
                ..
            } => {
                self.mobile_question_review(*turn_generation, request_id, review_hash)?;
                self.start_question_reply(request_id, reply.clone().into(), Some(device), cx)
                    .map_err(|error| MobileError::Invalid(error.into()))?;
                self.mark_question_prompt_sending(cx);
                Ok(MobileCommandOutcome::Accepted { queued: false })
            }
        }
    }
}

fn review_request(request: &DesktopPermissionRequest) -> Option<Review> {
    let mut review = Review {
        digest: Sha256::new(),
        prefix: Vec::new(),
        bytes: 0,
    };
    review.digest.update(request.tool.len().to_le_bytes());
    review.digest.update(request.tool.as_bytes());
    serde_json::to_writer(&mut review, &request.input).ok()?;
    Some(review)
}

fn mobile_question_text(text: &str) -> String {
    text.split('\n')
        .map(bruhs_models::secrets::review_text)
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn mobile_label(text: &str, budget: usize) -> String {
    let visible = bruhs_models::secrets::review_text(text);
    text_slice(&visible, 0, budget).map_or_else(|_| String::new(), |(s, _)| s.to_owned())
}
fn message_reference(message: &Message, index: usize) -> String {
    let mut hash = Sha256::new();
    hash.update(index.to_le_bytes());
    hash.update(message.role.as_str());
    hash.update(message.started_at_ms.unwrap_or_default().to_le_bytes());
    hash.update(message.content.len().to_le_bytes());
    hash.update(text_tail(&message.content, TEXT_BYTES).1.as_bytes());
    format!("{:x}", hash.finalize())
}
fn changed() -> MobileError {
    MobileError::Conflict(
        "This thread, message or request changed. Reload its current state before continuing."
            .into(),
    )
}

#[cfg(test)]
impl ChatView {
    pub(crate) fn seed_mobile_fixture(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.queue_held = true;
        self.input_state.update(cx, |input, cx| {
            input.set_value("laptop unsent draft", window, cx)
        });
        self.turn_generation = 17;
        self.pending_permission = Some(DesktopPermissionRequest {
            can_answer: false,
            id: "fixture-permission".into(),
            tool: "Read".into(),
            input: serde_json::json!({"file_path":"synthetic.rs"}),
        });
        let thread_id = ThreadId::try_from(self.thread_id.as_deref().unwrap()).unwrap();
        self.pending_secret = Some(bruhs_models::secrets::SecretRequest {
            id: "PRIVATE_PHONE_METADATA_CANARY".into(),
            thread_id,
            operation: bruhs_models::secrets::SecretOperation {
                name: "PRIVATE_PHONE_METADATA_CANARY".into(),
                reason: "Synthetic private form".into(),
                env_var: "SYNTHETIC_KEY".into(),
                command: vec!["fixture".into()],
            },
            cwd: "/private/fixture".into(),
            saved_available: false,
            storage_unavailable: false,
            connection: None,
            saved_source: None,
        });
        drop(self.render_secret_prompt(window, cx));
        self.secret_prompt
            .as_ref()
            .unwrap()
            .clone()
            .update(cx, |prompt, cx| prompt.set_mobile_fixture_value(window, cx));
    }
    pub(crate) fn mobile_fixture_change_permission(&mut self, large: bool) {
        self.pending_permission = Some(DesktopPermissionRequest {
            can_answer: false,
            id: "fixture-permission".into(),
            tool: "Read".into(),
            input: serde_json::json!({"file_path":if large{"x".repeat(TEXT_BYTES*2)}else{"changed.rs".into()}}),
        });
    }
    pub(crate) fn mobile_fixture_local_answer(&mut self, cx: &mut Context<'_, Self>) {
        self.respond_to_permission(false, cx);
    }
    pub(crate) fn mobile_fixture_advance_turn(&mut self) {
        self.turn_generation += 1;
    }
    pub(crate) fn mobile_fixture_draft(&self, cx: &App) -> String {
        self.input_state.read(cx).value().to_string()
    }
}
