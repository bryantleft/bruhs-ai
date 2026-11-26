use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use bruhs_models::logic::attachments::{MAX_FILES_PER_MESSAGE, TOO_MANY_FILES};
use gloo_timers::callback::Timeout;
use leptos::prelude::*;

use crate::components::file_attachments::{enqueue_browser_file_for, FileOffer, FileOffers};
use crate::state::ThreadNotice;
use crate::storage::composer_drafts::{self, DraftRecord};
use crate::task_saves::TaskSaves;

use super::{Draft, Drafts};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DraftStatus {
    Loading,
    Ready,
    Failed,
}

#[derive(Clone, Copy)]
pub(super) struct Persistence {
    states: RwSignal<BTreeMap<String, DraftStatus>>,
    latest: StoredValue<BTreeMap<String, Option<Arc<DraftRecord>>>>,
    dirty: RwSignal<BTreeSet<String>>,
    timer: StoredValue<Option<Timeout>, LocalStorage>,
    drops: RwSignal<BTreeMap<String, Vec<web_sys::File>>, LocalStorage>,
    saves: TaskSaves,
    notice: ThreadNotice,
}

impl Drafts {
    pub(crate) fn install() {
        let drafts = expect_context::<Self>();
        let persistence = Persistence {
            states: RwSignal::new(BTreeMap::new()),
            latest: StoredValue::new(BTreeMap::new()),
            dirty: RwSignal::new(BTreeSet::new()),
            timer: StoredValue::new_local(None),
            drops: RwSignal::new_local(BTreeMap::new()),
            saves: expect_context(),
            notice: expect_context(),
        };
        drafts.persistence.set_value(Some(persistence));
        drafts.files.on_change(Callback::new(move |id: String| {
            persistence.schedule(drafts, id);
        }));
        Effect::new(move |_| {
            let Some(id) = drafts.active.get() else {
                return;
            };
            if !persistence
                .states
                .with_untracked(|states| states.contains_key(&id))
            {
                persistence.load(drafts, id);
            }
        });
        Effect::new(move |_| {
            let Some(id) = drafts.active.get() else {
                return;
            };
            if !drafts.ready(&id)
                || drafts.locks.queue_editing.get()
                || drafts.locks.composing.get()
            {
                return;
            }
            drafts.text.track();
            drafts.chips.track();
            drafts.attachments.track();
            if persistence.body_changed(drafts, &id)
                || persistence
                    .dirty
                    .with_untracked(|dirty| dirty.contains(&id))
            {
                persistence.schedule(drafts, id);
            }
        });
    }

    pub(crate) fn register_new(self, id: &str) {
        if let Some(persistence) = self.persistence.get_value() {
            persistence.ready(id, None);
        }
    }

    pub(crate) fn attach_dropped_files(self, id: String, files: web_sys::FileList) {
        let Some(persistence) = self.persistence.get_value() else {
            return;
        };
        let exceeded = persistence
            .drops
            .try_update(|drops| {
                let pending = drops.entry(id.clone()).or_default();
                let room = MAX_FILES_PER_MESSAGE.saturating_sub(pending.len());
                pending.extend(
                    (0..files.length())
                        .take(room)
                        .filter_map(|index| files.get(index)),
                );
                files.length() as usize > room
            })
            .unwrap_or(false);
        if exceeded {
            persistence.notice.flash(TOO_MANY_FILES);
        }
        if self.ready(&id) {
            persistence.enqueue_drops(self, &id);
        } else if !persistence
            .states
            .with_untracked(|states| states.contains_key(&id))
        {
            persistence.load(self, id);
        }
    }

    pub(crate) fn status(self, id: &str) -> DraftStatus {
        self.persistence
            .get_value()
            .map_or(DraftStatus::Ready, |persistence| {
                persistence
                    .states
                    .with(|states| states.get(id).copied().unwrap_or(DraftStatus::Loading))
            })
    }

    pub(crate) fn ready(self, id: &str) -> bool {
        self.status(id) == DraftStatus::Ready
    }

    pub(crate) fn retry(self, id: String) {
        if let Some(persistence) = self.persistence.get_value() {
            if persistence
                .states
                .with_untracked(|states| states.get(&id) == Some(&DraftStatus::Failed))
            {
                persistence.load(self, id);
            }
        }
    }

    pub(crate) fn pending(self) -> bool {
        self.persistence.get_value().is_some_and(|persistence| {
            persistence.dirty.with(|dirty| !dirty.is_empty())
                || persistence.drops.with(|drops| !drops.is_empty())
        }) || self.active.with(|active| {
            active.as_deref().is_some_and(|id| {
                !self.ready(id)
                    && (!self.text.with(String::is_empty)
                        || !self.chips.with(Vec::is_empty)
                        || !self.attachments.with(Vec::is_empty))
            })
        }) || self.locks.queue_editing.get()
            || self.locks.composing.get()
            || self.files.has_unsaved_uploads()
    }
}

impl Persistence {
    fn enqueue_drops(self, drafts: Drafts, id: &str) {
        let files = self
            .drops
            .try_update(|drops| drops.remove(id).unwrap_or_default())
            .unwrap_or_default();
        for file in files {
            if let Some(message) = enqueue_browser_file_for(drafts.files, id, file) {
                self.notice.flash(message);
                break;
            }
        }
    }
    fn body_changed(self, drafts: Drafts, id: &str) -> bool {
        self.latest.with_value(|latest| {
            if let Some(Some(original)) = latest.get(id) {
                drafts
                    .text
                    .with_untracked(|text| text != &original.draft.text)
                    || drafts
                        .chips
                        .with_untracked(|chips| chips != &original.draft.chips)
                    || drafts
                        .attachments
                        .with_untracked(|attachments| attachments != &original.draft.attachments)
            } else {
                !drafts.text.with_untracked(String::is_empty)
                    || !drafts.chips.with_untracked(Vec::is_empty)
                    || !drafts.attachments.with_untracked(Vec::is_empty)
            }
        })
    }

    fn ready(self, id: &str, original: Option<Arc<DraftRecord>>) {
        self.latest.update_value(|latest| {
            latest.insert(id.to_owned(), original);
        });
        self.states.update(|states| {
            states.insert(id.to_owned(), DraftStatus::Ready);
        });
    }

    pub(super) fn forget(self, id: &str) {
        self.drops.update(|drops| {
            drops.remove(id);
        });
        self.states.update(|states| {
            states.remove(id);
        });
        self.latest.update_value(|latest| {
            latest.remove(id);
        });
        self.dirty.update(|dirty| {
            dirty.remove(id);
        });
    }

    fn load(self, drafts: Drafts, id: String) {
        self.states.update(|states| {
            states.insert(id.clone(), DraftStatus::Loading);
        });
        leptos::task::spawn_local(async move {
            let saved = composer_drafts::load(&id).await;
            if !self
                .states
                .try_with_untracked(|states| states.get(&id) == Some(&DraftStatus::Loading))
                .unwrap_or(false)
            {
                return;
            }
            let saved = match saved {
                Ok(saved) => saved,
                Err(error) => {
                    log::warn!("Saved composer draft loading failed: {error:?}");
                    self.states.update(|states| {
                        states.insert(id, DraftStatus::Failed);
                    });
                    return;
                }
            };
            batch(|| {
                let active = drafts
                    .active
                    .with_untracked(|active| active.as_ref() == Some(&id));
                let body = if active {
                    drafts.current()
                } else {
                    drafts
                        .parked
                        .with_value(|parked| parked.get(&id).cloned().unwrap_or_default())
                };
                let local = snapshot(&id, &body, drafts.files);
                if let Some(saved) = saved.as_ref().filter(|_| local.is_none()) {
                    drafts.restore_if_empty(
                        id.clone(),
                        active,
                        saved.draft.clone(),
                        saved.files.iter().cloned().map(FileOffer::from).collect(),
                    );
                }
                // A programmatic edit can arrive during loading. A different
                // saved version must win the compare, while the new edit stays
                // available through the writer's recovery interface.
                let original = if local.is_none() || local.as_deref() == saved.as_ref() {
                    saved.map(Arc::new)
                } else {
                    None
                };
                let needs_save = local.is_some() && local != original;
                self.ready(&id, original);
                if needs_save {
                    self.schedule(drafts, id.clone());
                }
                self.enqueue_drops(drafts, id.as_str());
            });
        });
    }

    fn schedule(self, drafts: Drafts, id: String) {
        if !self
            .states
            .with_untracked(|states| states.get(&id) == Some(&DraftStatus::Ready))
        {
            return;
        }
        self.dirty.update(|dirty| {
            dirty.insert(id);
        });
        if self.timer.with_value(Option::is_some) {
            return;
        }
        self.timer.set_value(Some(Timeout::new(180, move || {
            self.timer.set_value(None);
            let active = drafts.active.get_untracked();
            let editing = drafts.locks.queue_editing.get_untracked()
                || drafts.locks.composing.get_untracked();
            for id in self.dirty.get_untracked() {
                if active.as_ref() == Some(&id) {
                    if !editing {
                        self.flush(&id, &drafts.current(), drafts.files);
                    }
                } else {
                    drafts.parked.with_value(|parked| {
                        self.flush(
                            &id,
                            parked.get(&id).unwrap_or(&Draft::default()),
                            drafts.files,
                        );
                    });
                }
            }
        })));
    }

    pub(super) fn flush(self, id: &str, draft: &Draft, files: FileOffers) {
        let Some(original) = self.latest.with_value(|latest| latest.get(id).cloned()) else {
            return;
        };
        let current = snapshot(id, draft, files);
        if current != original {
            self.latest.update_value(|latest| {
                latest.insert(id.to_owned(), current.clone());
            });
            self.saves
                .edit_composer_draft(id.to_owned(), original, current);
        }
        self.dirty.update(|dirty| {
            dirty.remove(id);
        });
    }
}

fn snapshot(id: &str, draft: &Draft, files: FileOffers) -> Option<Arc<DraftRecord>> {
    let files = files.snapshot(id);
    if draft.is_empty() && files.is_empty() {
        return None;
    }
    Some(Arc::new(DraftRecord {
        thread_id: id.to_owned(),
        draft: draft.clone(),
        files,
    }))
}
