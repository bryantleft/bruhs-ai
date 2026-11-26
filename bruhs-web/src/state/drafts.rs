//! Keep a browser composer draft scoped to its conversation.

use std::collections::BTreeMap;

use bruhs_models::logic::{composer_text::TextAttachment, context_chip::ContextChip};
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::components::composer::{ComposerDraftSource, TextAttachments};
use crate::components::file_attachments::{FileOffer, FileOffers};

use super::{ChipRetargetGuard, ComposerChips, ComposerDraft};

mod persistence;
pub(crate) use persistence::DraftStatus;
use persistence::Persistence;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    pub text: String,
    pub chips: Vec<ContextChip>,
    pub attachments: Vec<TextAttachment>,
}

impl Draft {
    fn is_empty(&self) -> bool {
        self.text.is_empty() && self.chips.is_empty() && self.attachments.is_empty()
    }
}

#[derive(Clone, Copy)]
pub struct ComposerLocks {
    pub composing: RwSignal<bool>,
    pub queue_editing: RwSignal<bool>,
}

#[derive(Clone, Copy)]
pub struct Drafts {
    active: ReadSignal<Option<String>>,
    text: RwSignal<String>,
    chips: RwSignal<Vec<ContextChip>>,
    attachments: RwSignal<Vec<TextAttachment>>,
    parked: StoredValue<BTreeMap<String, Draft>>,
    source: ComposerDraftSource,
    retarget: StoredValue<bool>,
    files: FileOffers,
    locks: ComposerLocks,
    persistence: StoredValue<Option<Persistence>>,
}

impl Drafts {
    pub(super) fn provide(active: ReadSignal<Option<String>>) -> Self {
        let drafts = Self {
            active,
            text: RwSignal::new(String::new()),
            chips: RwSignal::new(Vec::new()),
            attachments: RwSignal::new(Vec::new()),
            parked: StoredValue::new(BTreeMap::new()),
            source: ComposerDraftSource(StoredValue::new(None)),
            retarget: StoredValue::new(false),
            files: FileOffers::new(active),
            locks: ComposerLocks {
                composing: RwSignal::new(false),
                queue_editing: RwSignal::new(false),
            },
            persistence: StoredValue::new(None),
        };
        provide_context(ComposerDraft(drafts.text));
        provide_context(ComposerChips(drafts.chips));
        provide_context(ChipRetargetGuard(drafts.retarget));
        provide_context(drafts.source);
        provide_context(TextAttachments(drafts.attachments));
        provide_context(drafts.files);
        provide_context(drafts.locks);
        provide_context(drafts);
        drafts
    }

    pub fn forget(self, id: &str) {
        if let Some(persistence) = self.persistence.get_value() {
            persistence.forget(id);
        }
        self.parked.update_value(|drafts| {
            drafts.remove(id);
        });
        self.files.forget(id);
    }

    pub fn restore_if_empty(
        self,
        id: String,
        is_active: bool,
        draft: Draft,
        files: Vec<FileOffer>,
    ) {
        if !self.files.is_empty_for(&id) {
            return;
        }
        if is_active {
            if !self.text.with_untracked(String::is_empty)
                || !self.chips.with_untracked(Vec::is_empty)
                || !self.attachments.with_untracked(Vec::is_empty)
            {
                return;
            }
            self.place(draft);
        } else if !self
            .parked
            .try_update_value(|parked| {
                if parked.contains_key(&id) {
                    return false;
                }
                parked.insert(id.clone(), draft);
                true
            })
            .unwrap_or(false)
        {
            return;
        }
        self.files.set_for(id, files);
    }

    fn place(self, draft: Draft) {
        self.retarget.set_value(true);
        self.chips.set(draft.chips);
        self.attachments.set(draft.attachments);
        self.text.set(draft.text);
    }

    fn switch(self, previous: Option<String>, next: Option<&str>) {
        if let Some(read) = self.source.0.get_value() {
            read.run(());
        }
        let outgoing = self.current();
        if let (Some(id), Some(persistence)) = (&previous, self.persistence.get_value()) {
            persistence.flush(id, &outgoing, self.files);
        }
        let incoming = self
            .parked
            .try_update_value(|parked| {
                if let Some(id) = previous {
                    if !outgoing.is_empty() {
                        parked.insert(id, outgoing);
                    }
                }
                next.and_then(|id| parked.remove(id)).unwrap_or_default()
            })
            .unwrap_or_default();
        self.place(incoming);
    }

    fn current(self) -> Draft {
        Draft {
            text: self.text.get_untracked(),
            chips: self.chips.get_untracked(),
            attachments: self.attachments.get_untracked(),
        }
    }
}

/// Restore the destination draft before callers write to the composer.
#[derive(Clone, Copy)]
pub struct ActiveThreadWriter {
    pub(super) active: ReadSignal<Option<String>>,
    pub(super) set_active: WriteSignal<Option<String>>,
    pub(super) drafts: Drafts,
}

impl ActiveThreadWriter {
    pub fn set(self, next: Option<String>) {
        let previous = self.active.get_untracked();
        if previous == next {
            return;
        }
        batch(|| {
            self.drafts.switch(previous, next.as_deref());
            self.set_active.set(next);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_restores_saved_drafts_before_new_composer_writes() {
        Owner::new().with(|| {
            let (active, set_active) = signal(Some("a".to_string()));
            let drafts = Drafts::provide(active);
            let writer = ActiveThreadWriter {
                active,
                set_active,
                drafts,
            };
            drafts.text.set("Original A".into());
            drafts.attachments.set(vec![TextAttachment {
                name: "Notes".into(),
                text: "Keep with A".into(),
            }]);
            writer.set(Some("b".into()));
            assert!(drafts.text.get_untracked().is_empty());
            assert!(drafts.attachments.with_untracked(Vec::is_empty));
            drafts.text.set("Draft B".into());
            writer.set(Some("a".into()));
            assert_eq!(drafts.text.get_untracked(), "Original A");
            assert_eq!(
                drafts
                    .attachments
                    .with_untracked(|items| items[0].text.clone()),
                "Keep with A"
            );
            writer.set(Some("b".into()));
            assert_eq!(drafts.text.get_untracked(), "Draft B");
            assert!(drafts.attachments.with_untracked(Vec::is_empty));
        });
    }
}
