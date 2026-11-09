//! App-owned transcript refresh: route changes do not cancel durable cache acknowledgment.
use crate::state::{Meeting, TranscriptSegment};
use crate::storage::transcripts::{self, Snapshot};
use leptos::prelude::*;
use std::collections::BTreeMap;
mod stream;

#[derive(Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub loading: bool,
    pub warning: Option<String>,
    again: bool,
}

#[cfg(test)]
mod tests;

#[derive(Clone, Copy)]
pub struct TranscriptSync {
    statuses: RwSignal<BTreeMap<String, Status>>,
    meetings: ReadSignal<Vec<Meeting>>,
    set_meetings: WriteSignal<Vec<Meeting>>,
    segments: ReadSignal<BTreeMap<String, Vec<TranscriptSegment>>>,
    set_segments: WriteSignal<BTreeMap<String, Vec<TranscriptSegment>>>,
    stream: StoredValue<stream::StreamBuffer>,
    live: ReadSignal<Option<String>>,
    set_live: WriteSignal<Option<String>>,
    banners: crate::components::banner::BannerStackSignal,
}

impl TranscriptSync {
    pub fn new(
        meetings: ReadSignal<Vec<Meeting>>,
        set_meetings: WriteSignal<Vec<Meeting>>,
        segments: ReadSignal<BTreeMap<String, Vec<TranscriptSegment>>>,
        set_segments: WriteSignal<BTreeMap<String, Vec<TranscriptSegment>>>,
        live: ReadSignal<Option<String>>,
        set_live: WriteSignal<Option<String>>,
        banners: crate::components::banner::BannerStackSignal,
    ) -> Self {
        Self {
            statuses: RwSignal::new(BTreeMap::new()),
            meetings,
            set_meetings,
            segments,
            set_segments,
            stream: StoredValue::new(stream::StreamBuffer::default()),
            live,
            set_live,
            banners,
        }
    }

    pub fn status(self, id: &str) -> Status {
        self.statuses
            .with(|statuses| statuses.get(id).cloned().unwrap_or_default())
    }

    pub fn report(self, id: &str, warning: String) {
        self.statuses
            .update(|statuses| statuses.entry(id.into()).or_default().warning = Some(warning));
    }

    pub fn refresh(self, id: String) {
        if self
            .statuses
            .with_untracked(|statuses| statuses.get(&id).is_some_and(|status| status.loading))
        {
            self.statuses
                .update(|statuses| statuses.entry(id).or_default().again = true);
            return;
        }
        self.statuses
            .update(|statuses| statuses.entry(id.clone()).or_default().loading = true);
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                self.statuses
                    .update(|statuses| statuses.entry(id.clone()).or_default().again = false);
                let warning = match self.refresh_once(&id).await {
                    Ok(warning) => warning,
                    Err(error) => Some(format!("Transcript refresh did not finish. {error} Existing content will not be replaced; retry loading the transcript.")),
                };
                let again = self.statuses.with_untracked(|statuses| {
                    statuses.get(&id).is_some_and(|status| status.again)
                });
                self.statuses.update(|statuses| {
                    let status = statuses.entry(id.clone()).or_default();
                    status.warning = warning;
                    status.loading = again;
                });
                if !again {
                    break;
                }
            }
        });
    }

    fn publish_meeting(self, meeting: Meeting) {
        let id = meeting.id.clone();
        self.set_meetings.update(|meetings| {
            if let Some(current) = meetings.iter_mut().find(|current| current.id == meeting.id) {
                if meeting.updated_at > current.updated_at {
                    *current = meeting;
                }
            } else {
                meetings.push(meeting);
            }
        });
        let ended = self.meetings.with_untracked(|all| {
            all.iter().any(|meeting| {
                meeting.id == id && meeting.status != crate::state::MeetingStatus::Live
            })
        });
        if ended
            && self
                .live
                .with_untracked(|live| live.as_deref() == Some(&id))
        {
            self.set_live.set(None);
        }
    }

    pub async fn cache_meeting(self, meeting: Meeting) {
        let id = meeting.id.clone();
        match self.acknowledge_meeting(meeting).await {
            Ok(()) => {},
            Err(error) => self.report(&id, format!("The server saved the meeting change, but this device could not cache it. {error} Retry loading the transcript.")),
        }
    }

    pub async fn acknowledge_meeting(self, meeting: Meeting) -> Result<(), String> {
        let newer_is_open = || {
            self.meetings.with_untracked(|all| {
                all.iter().any(|current| {
                    current.id == meeting.id && current.updated_at > meeting.updated_at
                })
            })
        };
        if newer_is_open() {
            return Err("A newer meeting version is already open.".into());
        }
        let saved = transcripts::cache_meeting(meeting.clone())
            .await
            .map_err(|error| error.to_string())?;
        if saved != meeting || newer_is_open() {
            return Err("A newer saved meeting exists; load it before retrying.".into());
        }
        self.publish_meeting(saved);
        Ok(())
    }

    async fn refresh_once(self, id: &str) -> Result<Option<String>, String> {
        self.flush_stream(id).await?;
        let mut receipt = None;
        self.stream
            .update_value(|buffer| receipt = Some(buffer.receipt(id)));
        let mut receipt = receipt.ok_or("Transcript cache is unavailable.")?;
        let before_load = self
            .meetings
            .with_untracked(|all| all.iter().find(|meeting| meeting.id == id).cloned());
        let expected = transcripts::load(id)
            .await
            .map_err(|error| error.to_string())?;
        self.check_readback(id, &receipt, &before_load)?;
        if let Some(meeting) = expected.meeting.clone() {
            self.publish_meeting(meeting);
        }
        self.set_segments.update(|all| {
            if all.get(id).is_none_or(Vec::is_empty) && !expected.segments.is_empty() {
                all.insert(id.into(), expected.segments.clone());
                self.stream.update_value(|buffer| buffer.replaced(id));
            }
        });
        self.stream
            .update_value(|buffer| receipt = buffer.receipt(id));
        let baseline = self
            .meetings
            .with_untracked(|all| all.iter().find(|meeting| meeting.id == id).cloned());
        let request = async {
            let (meeting, transcript) = futures_util::join!(
                crate::api::get_meeting(id),
                crate::api::fetch_transcript(id)
            );
            Ok::<_, String>((
                meeting
                    .map_err(|error| error.to_string())?
                    .ok_or("Meeting is unavailable on the server.")?,
                transcript.map_err(|error| error.to_string())?,
            ))
        };
        let (meeting, reply) = match futures_util::future::select(
            Box::pin(request),
            Box::pin(gloo_timers::future::TimeoutFuture::new(15_000)),
        )
        .await
        {
            futures_util::future::Either::Left((result, _)) => result?,
            futures_util::future::Either::Right(_) => {
                return Err("The server request timed out.".into())
            }
        };
        self.check_readback(id, &receipt, &baseline)?;
        // Forget authorized removals before this snapshot is validated. IndexedDB
        // will not drop a finalized row that the reply simply omits, and forgetting
        // changes the receipt, so a cleaned pass retries against a fresh baseline.
        let removals = crate::api::removed_transcript_segments(id)
            .await
            .map_err(|error| error.to_string())?;
        let mut cleaned = false;
        for segment_id in removals {
            let (changed, removed) = self.forget_chunk(id, &segment_id);
            match transcripts::forget_segment(id, &segment_id).await {
                Ok(deleted) => cleaned |= changed || deleted || removed.is_some(),
                Err(error) => {
                    if let Some(segment) = removed {
                        self.restore_chunk(segment);
                    }
                    return Err(error.to_string());
                }
            }
        }
        if cleaned {
            self.statuses.update(|statuses| {
                statuses.entry(id.to_owned()).or_default().again = true;
            });
            return Ok(None);
        }
        if baseline.as_ref().is_some_and(|current| {
            current.updated_at > meeting.updated_at
                || (current.updated_at == meeting.updated_at && current != &meeting)
        }) {
            return Err("A newer or conflicting meeting version is already open.".into());
        }
        let mut incoming = Snapshot {
            meeting: Some(meeting),
            segments: reply.segments,
        };
        self.segments
            .with_untracked(|all| {
                bruhs_models::validate_transcript_snapshot(
                    id,
                    all.get(id).map_or(&[], Vec::as_slice),
                    &mut incoming.segments,
                )
            })
            .map_err(|error| error.to_string())?;
        let gate = self.stream.with_value(|buffer| buffer.gate.clone());
        let _guard = gate.lock().await;
        self.check_readback(id, &receipt, &baseline)?;
        let saved = transcripts::cache(id, &expected, incoming)
            .await
            .map_err(|error| error.to_string())?;
        // Speech can arrive even while an IndexedDB transaction is committing.
        self.check_readback(id, &receipt, &baseline)?;
        if let Some(meeting) = saved.meeting {
            self.publish_meeting(meeting);
        }
        self.set_segments.update(|all| {
            all.insert(id.into(), saved.segments);
        });
        self.stream.update_value(|buffer| buffer.replaced(id));
        Ok(reply.saved_fallback.then(|| "Showing the saved transcript. Capture refresh is unavailable; retry when connected.".into()))
    }

    fn check_readback(
        self,
        id: &str,
        receipt: &std::sync::Arc<()>,
        baseline: &Option<Meeting>,
    ) -> Result<(), String> {
        if !self.stream.with_value(|buffer| buffer.current(id, receipt))
            || !self.meetings.with_untracked(|all| {
                all.iter().find(|meeting| meeting.id == id) == baseline.as_ref()
            })
        {
            return Err("New speech or a meeting edit arrived during the request. Current content is retained.".into());
        }
        Ok(())
    }
}
