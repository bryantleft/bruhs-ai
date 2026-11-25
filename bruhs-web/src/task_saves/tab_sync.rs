//! Same-origin task updates. Notifications carry keys; IndexedDB supplies the
//! committed records, even when notices arrive out of order.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use leptos::leptos_dom::helpers::window_event_listener;
use leptos::prelude::*;
use wasm_bindgen::{closure::Closure, JsCast};
use web_sys::{BroadcastChannel, MessageEvent};

use super::{Queue, TaskSaves};
use crate::state::{SetWorkItemsSignal, WorkItem, WorkItemsSignal};
use crate::storage;

struct Subscription {
    channel: BroadcastChannel,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.channel.set_onmessage(None);
        self.channel.close();
    }
}

thread_local! {
    static SUBSCRIPTION: RefCell<Option<Subscription>> = const { RefCell::new(None) };
}

pub(crate) fn publish_task_changes<'a>(ids: impl Iterator<Item = &'a str>) {
    let ids = ids.collect::<Vec<_>>();
    if ids.is_empty() {
        return;
    }
    SUBSCRIPTION.with_borrow(|subscription| {
        if let Some(subscription) = subscription {
            let message = serde_wasm_bindgen::to_value(&ids).and_then(|value| {
                subscription
                    .channel
                    .post_message(&value)
                    .map_err(Into::into)
            });
            if let Err(error) = message {
                log::warn!("Could not notify other tabs of saved tasks: {error}");
            }
        }
    });
}

#[derive(Default)]
struct Refresh {
    ids: BTreeSet<String>,
    snapshot: bool,
}

pub(super) fn listen(saves: TaskSaves, ready: RwSignal<bool>) {
    let items = expect_context::<WorkItemsSignal>().0;
    let set_items = expect_context::<SetWorkItemsSignal>().0;
    let refresh = RwSignal::new(Refresh::default());
    let reading = RwSignal::new(false);

    match BroadcastChannel::new("bruhs-work-items") {
        Ok(channel) => {
            let on_message = Closure::new(move |event: MessageEvent| {
                if let Ok(ids) = serde_wasm_bindgen::from_value::<Vec<String>>(event.data()) {
                    refresh.update(|refresh| refresh.ids.extend(ids));
                }
            });
            channel.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
            SUBSCRIPTION.with_borrow_mut(|slot| {
                *slot = Some(Subscription {
                    channel,
                    _on_message: on_message,
                });
            });
        }
        Err(error) => log::warn!("Live task updates between tabs are unavailable: {error:?}"),
    }

    // A back/forward-cache entry may have missed notifications while frozen.
    let page_hidden = RwSignal::new(false);
    let hide = window_event_listener(leptos::ev::pagehide, move |_| page_hidden.set(true));
    let show = window_event_listener(leptos::ev::pageshow, move |_| {
        if page_hidden.get_untracked() {
            page_hidden.set(false);
            refresh.update(|refresh| refresh.snapshot = true);
        }
    });
    on_cleanup(move || {
        hide.remove();
        show.remove();
        SUBSCRIPTION.with_borrow_mut(|slot| *slot = None);
    });

    Effect::new(move |_| {
        if !ready.get()
            || reading.get()
            || page_hidden.get()
            || saves.day_plan.with(|state| state.busy)
            || !refresh.with(|refresh| {
                refresh.snapshot
                    || (!refresh.ids.is_empty()
                        && saves
                            .queue
                            .with(|queue| refresh.ids.iter().any(|id| !queue.task_pending(id))))
            })
        {
            return;
        }
        reading.set(true);
        let pending_at_start = saves.queue.with_untracked(Queue::pending_task_ids);
        let next = refresh
            .try_update(|refresh| Refresh {
                ids: refresh
                    .ids
                    .extract_if(.., |id| !pending_at_start.contains(id))
                    .collect(),
                snapshot: std::mem::take(&mut refresh.snapshot),
            })
            .unwrap_or_default();
        wasm_bindgen_futures::spawn_local(async move {
            let baseline = items.with_untracked(|items| {
                items
                    .iter()
                    .filter(|item| next.snapshot || next.ids.contains(&item.id))
                    .map(|item| (item.id.clone(), item.clone()))
                    .collect::<BTreeMap<_, _>>()
            });
            let changed = if next.snapshot {
                storage::tasks::load().await.map(|snapshot| {
                    let mut changed = baseline
                        .keys()
                        .cloned()
                        .map(|id| (id, None))
                        .collect::<BTreeMap<_, _>>();
                    changed.extend(
                        snapshot
                            .into_iter()
                            .map(|item| (item.id.clone(), Some(item))),
                    );
                    changed
                })
            } else {
                storage::tasks::load_changed(&next.ids).await
            };
            if reading.try_get_untracked().is_none() {
                return;
            }
            match changed {
                Ok(mut changed) => {
                    if saves.day_plan.with_untracked(|state| state.busy) {
                        refresh.update(|refresh| {
                            refresh.ids.extend(next.ids);
                            refresh.snapshot |= next.snapshot;
                        });
                    } else {
                        let mut deferred = BTreeSet::new();
                        saves.queue.with_untracked(|queue| {
                            changed.retain(|id, _| {
                                if pending_at_start.contains(id) || queue.task_pending(id) {
                                    deferred.insert(id.clone());
                                    false
                                } else {
                                    true
                                }
                            });
                        });
                        if !deferred.is_empty() {
                            refresh.update(|refresh| refresh.ids.extend(deferred));
                        }
                        set_items.maybe_update(|items| apply_changes(items, &baseline, changed));
                    }
                }
                Err(error) => {
                    log::warn!("Could not refresh tasks from IndexedDB: {error:?}");
                    saves.banners.push(
                        crate::components::banner::BannerKind::Warning,
                        "Could not load changes from another tab. Reload to check for newer tasks.",
                        None,
                    );
                }
            }
            reading.set(false);
        });
    });
}

fn apply_changes(
    items: &mut Vec<WorkItem>,
    baseline: &BTreeMap<String, WorkItem>,
    mut changed: BTreeMap<String, Option<WorkItem>>,
) -> bool {
    let mut updated = false;
    let mut sort = false;
    items.retain_mut(|item| {
        let Some(incoming) = changed.remove(&item.id) else {
            return true;
        };
        // A local edit may have landed between the read and its completion.
        if baseline.get(&item.id) != Some(item) || incoming.as_ref() == Some(item) {
            return true;
        }
        updated = true;
        if let Some(incoming) = incoming {
            sort |= item.created_at != incoming.created_at;
            *item = incoming;
            true
        } else {
            false
        }
    });
    let previous_len = items.len();
    items.extend(changed.into_iter().filter_map(|(id, item)| {
        // Do not resurrect a row deleted while the storage read was pending.
        if baseline.contains_key(&id) {
            None
        } else {
            item
        }
    }));
    sort |= previous_len != items.len();
    if sort {
        items.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        updated = true;
    }
    updated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_preserves_edits_and_deletions_made_after_the_read_started() {
        let original = WorkItem::new_at("Original", 1);
        let deleted = WorkItem::new_at("Deleted during the read", 2);
        let baseline = [original.clone(), deleted.clone()]
            .into_iter()
            .map(|item| (item.id.clone(), item))
            .collect();
        let mut local = original.clone();
        local.title = "New local edit".into();
        let mut peer = original;
        peer.title = "Earlier remote edit".into();
        let changed = [
            (peer.id.clone(), Some(peer)),
            (deleted.id.clone(), Some(deleted)),
        ]
        .into();
        let mut items = vec![local.clone()];

        assert!(!apply_changes(&mut items, &baseline, changed));
        assert_eq!(items, vec![local]);
    }

    #[test]
    fn refresh_merges_new_rows_and_tombstones_in_creation_order() {
        let first = WorkItem::new_at("First", 1);
        let removed = WorkItem::new_at("Removed", 2);
        let last = WorkItem::new_at("Last", 3);
        let mut items = vec![removed.clone(), last.clone()];
        let baseline = items
            .iter()
            .map(|item| (item.id.clone(), item.clone()))
            .collect();
        let changed = [(first.id.clone(), Some(first.clone())), (removed.id, None)].into();

        assert!(apply_changes(&mut items, &baseline, changed));
        assert_eq!(items, vec![first, last]);
    }
}
