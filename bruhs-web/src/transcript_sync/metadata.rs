//! Resolve a task's meeting without requiring the Meetings page or loading speech.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MeetingLookup {
    Loading,
    Ready,
    Missing,
    Failed(String),
}

impl TranscriptSync {
    pub fn meeting_lookup(self, id: &str) -> Option<MeetingLookup> {
        self.meeting_lookups.with(|all| all.get(id).cloned())
    }

    pub fn ensure_meeting_metadata(self, id: String) {
        if !self
            .meeting_lookups
            .with_untracked(|all| all.contains_key(&id))
        {
            self.refresh_meeting_metadata(id);
        }
    }

    /// App ownership coalesces reads across task edits and route changes.
    pub fn refresh_meeting_metadata(self, id: String) {
        if self
            .meeting_lookups
            .with_untracked(|all| matches!(all.get(&id), Some(MeetingLookup::Loading)))
        {
            return;
        }
        self.meeting_lookups.update(|all| {
            all.insert(id.clone(), MeetingLookup::Loading);
        });
        wasm_bindgen_futures::spawn_local(async move {
            // A saved meeting stays usable when the companion is offline.
            if let Ok(Some(meeting)) = transcripts::load_meeting(&id).await {
                self.publish_meeting(meeting);
            }
            let result = match futures_util::future::select(
                Box::pin(crate::api::get_meeting(&id)),
                Box::pin(gloo_timers::future::TimeoutFuture::new(15_000)),
            )
            .await
            {
                futures_util::future::Either::Left((result, _)) => {
                    result.map_err(|error| error.to_string())
                }
                futures_util::future::Either::Right(_) => {
                    Err("The meeting lookup timed out.".into())
                }
            };
            let status = match result {
                Ok(Some(meeting)) => match self.acknowledge_meeting(meeting).await {
                    Ok(()) => MeetingLookup::Ready,
                    Err(error) => MeetingLookup::Failed(error),
                },
                Ok(None) => MeetingLookup::Missing,
                Err(error) => MeetingLookup::Failed(error),
            };
            self.meeting_lookups.update(|all| {
                all.insert(id, status);
            });
        });
    }
}
