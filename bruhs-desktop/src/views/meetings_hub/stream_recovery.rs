//! Read-only reconciliation for the two active transcript contexts after a gap.
use super::*;
use meeting_cache::TranscriptReceipt;

#[derive(Default)]
pub(super) struct Recovery {
    pub last_requested: Option<String>,
    in_flight: bool,
    again: bool,
}

struct Readback {
    id: String,
    baseline: Option<Meeting>,
    receipt: TranscriptReceipt,
    meeting: Result<Option<Meeting>, ServerError>,
    segments: Result<Vec<TranscriptSegment>, ServerError>,
    removed: Result<Vec<String>, ServerError>,
}

impl MeetingsHub {
    pub(super) fn recover_meeting_stream(&mut self, _cx: &mut Context<'_, Self>) {
        if self.stream_recovery.in_flight {
            self.stream_recovery.again = true;
            return;
        }
        // Reconnects must not download every historical transcript or restart work.
        let ids: HashSet<_> = self
            .live_meeting_id
            .iter()
            .chain(self.stream_recovery.last_requested.iter())
            .cloned()
            .collect();
        if ids.is_empty() {
            return;
        }
        self.stream_recovery.in_flight = true;
        self.stream_recovery.again = false;
        let contexts: Vec<_> = ids
            .into_iter()
            .map(|id| {
                let baseline = self.meeting(&id).cloned();
                let receipt = self.transcript_read_receipt(&id);
                (id, baseline, receipt)
            })
            .collect();
        let client = self.client.clone();
        self.run(async move {
            let mut requests = tokio::task::JoinSet::new();
            for (id, baseline, receipt) in contexts {
                let client = client.clone();
                requests.spawn(async move {
                    let (meeting, segments, removed) = tokio::join!(
                        client.get_meeting(&id),
                        client.list_segments(&id),
                        client.removed_segment_ids(&id),
                    );
                    Readback { id, baseline, receipt, meeting, segments, removed }
                });
            }
            let mut results = Vec::new();
            while let Some(result) = requests.join_next().await { results.push(result); }
            results
        }, |hub, results, cx| {
            for result in results {
                let result = result.map_err(|_| "Meeting recovery task did not finish.".to_string())
                    .and_then(|reply| hub.apply_stream_readback(reply, cx));
                if let Err(error) = result {
                    hub.notify_notice(format!("Live transcript recovery did not finish. {error} Current context is retained; reopen the meeting to retry."), true, cx);
                }
            }
            hub.stream_recovery.in_flight = false;
            if std::mem::take(&mut hub.stream_recovery.again) { hub.recover_meeting_stream(cx); }
            cx.notify();
        });
    }

    fn apply_stream_readback(
        &mut self,
        reply: Readback,
        cx: &mut Context<'_, Self>,
    ) -> Result<(), String> {
        let meeting = reply
            .meeting
            .map_err(|error| error.to_string())?
            .ok_or("The recording is unavailable on the server.")?;
        if meeting.id != reply.id {
            return Err("The server returned another recording.".into());
        }
        meeting_cache::validate_meeting(&meeting).map_err(str::to_owned)?;
        if self.meeting(&reply.id) != reply.baseline.as_ref()
            || !self.transcript_receipt_current(&reply.id, &reply.receipt)
        {
            return Err("The meeting or transcript changed during the request.".into());
        }
        let mut segments = reply.segments.map_err(|error| error.to_string())?;
        let removed = reply.removed.map_err(|error| error.to_string())?;
        for segment_id in removed {
            self.forget_transcript_chunk(&reply.id, &segment_id, cx);
        }
        bruhs_models::validate_transcript_snapshot(
            &reply.id,
            self.segments.get(&reply.id).map_or(&[], Vec::as_slice),
            &mut segments,
        )
        .map_err(|error| error.to_string())?;
        // Preflight both records before publishing either; no await separates updates.
        let metadata_changed = self.cache_meeting(&meeting).map_err(str::to_owned)?;
        let ended = meeting.status != MeetingStatus::Live;
        self.replace_meeting(meeting, cx);
        self.apply_transcript_snapshot(reply.id.clone(), segments, cx)
            .map_err(str::to_owned)?;
        if metadata_changed {
            cx.emit(HubEvent::MeetingsChanged);
        }
        if ended && self.live_meeting_id.as_deref() == Some(&reply.id) {
            self.stop_local_focus("Local app context is off: meeting ended.", cx);
            self.live_meeting_id = None;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
