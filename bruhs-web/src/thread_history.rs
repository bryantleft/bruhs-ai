//! Only the current activation may publish a saved conversation snapshot.
use leptos::prelude::*;

use crate::state::{AgentMessage, StreamBuffer};
pub(crate) mod focus;

#[derive(Clone, Debug, PartialEq)]
struct Request {
    thread_id: String,
    token: uuid::Uuid,
}

#[derive(Clone, Debug, Default, PartialEq)]
enum State {
    #[default]
    Ready,
    Loading(Request),
    Failed(Request),
}

impl State {
    fn accepts(&self, request: &Request, active: Option<&str>) -> bool {
        matches!(self, Self::Loading(current) if current == request)
            && active == Some(request.thread_id.as_str())
    }

    fn reason(&self, active: Option<&str>) -> Option<&'static str> {
        match self {
            Self::Loading(request) if active == Some(request.thread_id.as_str()) => {
                Some("Loading saved conversation…")
            }
            Self::Failed(request) if active == Some(request.thread_id.as_str()) => {
                Some("Saved conversation could not be loaded. Existing history was left unchanged.")
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
pub struct ThreadHistory(RwSignal<State>, crate::task_saves::TaskSaves);

impl ThreadHistory {
    pub fn install() {
        provide_context(Self(RwSignal::new(State::Ready), expect_context()));
    }

    pub fn reason(self, active: Option<&str>) -> Option<&'static str> {
        self.0.with(|state| state.reason(active))
    }

    pub fn blocked(self, active: &str) -> bool {
        self.0
            .with_untracked(|state| state.reason(Some(active)).is_some())
    }

    pub fn load(
        self,
        id: String,
        active: ReadSignal<Option<String>>,
        messages: WriteSignal<Vec<AgentMessage>>,
        buffer: RwSignal<StreamBuffer>,
    ) {
        // Invalidates older requests even when returning to the same thread.
        let request = Request {
            thread_id: id.clone(),
            token: uuid::Uuid::new_v4(),
        };
        self.0.set(State::Loading(request.clone()));
        let buffered = buffer.with_untracked(|buffer| {
            (buffer.thread_id.as_deref() == Some(id.as_str())).then(|| buffer.messages.clone())
        });
        if let Some(buffered) = buffered {
            messages.set(buffered);
            self.0.set(State::Ready);
            return;
        }
        if let Some(pending) = self.1.pending_conversation(&id) {
            messages.set(pending);
            self.0.set(State::Ready);
            return;
        }
        messages.set(Vec::new());
        leptos::task::spawn_local(async move {
            let loaded = crate::storage::load_messages(&id).await;
            if !self.0.with_untracked(|state| {
                active.with_untracked(|active| state.accepts(&request, active.as_deref()))
            }) {
                return;
            }
            match loaded {
                Ok(loaded) => {
                    let loaded = self.1.loaded_conversation(&id, loaded);
                    let restore_focus = focus::retain_composer_focus(active, id);
                    messages.set(loaded);
                    self.0.set(State::Ready);
                    restore_focus();
                }
                Err(error) => {
                    log::warn!("Saved conversation loading failed: {error:?}");
                    self.0.set(State::Failed(request));
                }
            }
        });
    }
}

#[component]
pub fn ThreadHistoryStatus() -> impl IntoView {
    let history = expect_context::<ThreadHistory>();
    let active = expect_context::<crate::state::ActiveThreadId>().0;
    let set_active = expect_context::<crate::state::SetActiveThreadId>().0;
    let messages = expect_context::<crate::state::SetMessagesSignal>().0;
    let crate::state::StreamBufferSignal(buffer) = expect_context();
    let crate::state::UnreadThreads(unread) = expect_context();
    view! {
        {move || active.with(|id| history.reason(id.as_deref())).map(|reason| view! {
            <section aria-label="Conversation loading status" role="status"
                class="flex shrink-0 flex-wrap items-center gap-2 border-b border-longan-700 px-3 py-2 text-body-sm text-rambutan-300 sm:px-5">
                <span>{reason}</span>
                <Show when=move || history.0.with(|state| matches!(state, State::Failed(_)))>
                    <button type="button" class="min-h-11 rounded-grape px-2 text-label-sm text-persimmon-300 hover:bg-longan-700 focus-visible:outline-2 focus-visible:outline-persimmon-400 cursor-pointer"
                        on:click=move |_| {
                            if let Some(id) = active.get_untracked() {
                                crate::state::activate_thread(history, active, id, set_active, messages, buffer, unread);
                            }
                        }>"Retry loading conversation"</button>
                </Show>
            </section>
        })}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(id: &str) -> Request {
        Request {
            thread_id: id.into(),
            token: uuid::Uuid::new_v4(),
        }
    }

    #[test]
    fn only_current_request_for_active_thread_can_publish() {
        let first = request("a");
        let latest = request("a");
        let state = State::Loading(latest.clone());
        assert!(state.accepts(&latest, Some("a")));
        assert!(!state.accepts(&first, Some("a")));
        assert!(!state.accepts(&latest, Some("b")));
        assert!(!state.accepts(&latest, None));
    }

    #[test]
    fn completed_or_failed_request_cannot_publish_again() {
        let request = request("a");
        assert!(!State::Ready.accepts(&request, Some("a")));
        assert!(!State::Failed(request.clone()).accepts(&request, Some("a")));
    }

    #[test]
    fn loading_and_failure_block_only_their_conversation() {
        for state in [State::Loading(request("a")), State::Failed(request("a"))] {
            assert!(state.reason(Some("a")).is_some());
            assert!(state.reason(Some("new")).is_none());
            assert!(state.reason(None).is_none());
        }
        assert!(State::Ready.reason(Some("a")).is_none());
    }
}
