use super::*;
use crate::meetings::{
    person_changes::next_version, MeetingRecords, PeopleStore, SpeakerLinkJobs,
    SummaryGenerationStore,
};
use bruhs_models::{
    Meeting, MeetingSummary, PersonObservation, PersonProfile, Speaker, TranscriptSegment,
};
use std::collections::HashMap;

#[async_trait::async_trait]
impl MeetingRecords for SqliteMeetingStore {
    async fn apply_capture_failure(
        &self,
        receipt: &bruhs_models::CaptureFailureReceipt,
    ) -> MeetingStoreResult<bool> {
        let receipt = receipt.clone();
        self.call(move |connection| {
            write(connection, |tx| {
                let mut meeting = records::required(
                    records::meeting(tx, &receipt.meeting_id)?,
                    &receipt.meeting_id,
                )?;
                let changed = crate::meetings::capture_failures::apply(&mut meeting, &receipt)?;
                if changed {
                    records::save_meeting(tx, &meeting)?;
                }
                Ok(changed)
            })
        })
        .await
    }

    async fn append_local_focus(
        &self,
        observation: bruhs_models::LocalFocusObservation,
    ) -> MeetingStoreResult<()> {
        self.call(move |connection| write(connection, |tx| local_focus::append(tx, &observation)))
            .await
    }

    async fn list_local_focus(
        &self,
        meeting_id: &str,
    ) -> MeetingStoreResult<Vec<bruhs_models::LocalFocusObservation>> {
        let id = meeting_id.to_owned();
        self.call(move |connection| local_focus::list(connection, &id))
            .await
    }

    async fn create_meeting(&self, meeting: Meeting) -> MeetingStoreResult<Meeting> {
        self.call(move |connection| {
            write(connection, |tx| {
                if let Some(current) = records::meeting(tx, &meeting.id)? {
                    if current != meeting {
                        return Err(MeetingStoreError::Conflict(
                            "Meeting already exists.".into(),
                        ));
                    }
                    return Ok(current);
                }
                records::save_meeting(tx, &meeting)?;
                Ok(meeting)
            })
        })
        .await
    }

    async fn get_meeting(&self, id: &str) -> MeetingStoreResult<Option<Meeting>> {
        let id = id.to_owned();
        self.call(move |connection| records::meeting(connection, &id))
            .await
    }

    async fn list_meetings(&self) -> MeetingStoreResult<Vec<Meeting>> {
        self.call(|connection| {
            all(
                connection,
                "SELECT id,payload FROM meetings ORDER BY started DESC,id",
                |id, value: &Meeting| value.id == id,
            )
        })
        .await
    }

    async fn upsert_meeting(&self, meeting: Meeting) -> MeetingStoreResult<Meeting> {
        self.call(move |connection| {
            write(connection, |tx| {
                if let Some(current) = records::meeting(tx, &meeting.id)? {
                    if current != meeting {
                        return Err(MeetingStoreError::Conflict(
                            "Meeting already exists; use a field-specific update.".into(),
                        ));
                    }
                    return Ok(current);
                }
                records::save_meeting(tx, &meeting)?;
                Ok(meeting)
            })
        })
        .await
    }

    async fn patch_meeting(
        &self,
        id: &bruhs_models::ids::MeetingId,
        patch: crate::meetings::MeetingPatch,
    ) -> MeetingStoreResult<Option<Meeting>> {
        let id = id.as_str();
        let id = id.to_owned();
        self.call(move |connection| {
            write(connection, |tx| {
                records::edit_meeting(tx, &id, |meeting| {
                    patch.apply(meeting);
                    Ok(())
                })
            })
        })
        .await
    }

    async fn patch_meeting_summary(
        &self,
        id: &bruhs_models::ids::MeetingId,
        summary: MeetingSummary,
    ) -> MeetingStoreResult<Option<Meeting>> {
        let id = id.as_str();
        let id = id.to_owned();
        self.call(move |connection| {
            write(connection, |tx| {
                records::edit_meeting(tx, &id, |meeting| {
                    meeting.summary = Some(summary);
                    Ok(())
                })
            })
        })
        .await
    }

    async fn patch_speaker_label(
        &self,
        meeting_id: &bruhs_models::ids::MeetingId,
        speaker_id: &bruhs_models::ids::SpeakerId,
        label: String,
    ) -> MeetingStoreResult<Meeting> {
        let meeting_id = meeting_id.as_str();
        let speaker_id = speaker_id.as_str();
        let (meeting_id, speaker_id) = (meeting_id.to_owned(), speaker_id.to_owned());
        self.call(move |connection| {
            write(connection, |tx| {
                records::required(
                    records::edit_meeting(tx, &meeting_id, |meeting| {
                        let speaker = meeting
                            .speakers
                            .iter_mut()
                            .find(|speaker| speaker.id == speaker_id)
                            .ok_or_else(|| MeetingStoreError::SpeakerNotFound {
                                meeting_id: meeting_id.clone(),
                                speaker_id,
                            })?;
                        speaker.label = label;
                        Ok(())
                    })?,
                    &meeting_id,
                )
            })
        })
        .await
    }

    async fn patch_meeting_thread_id(
        &self,
        meeting_id: &bruhs_models::ids::MeetingId,
        thread_id: &bruhs_models::ids::ThreadId,
    ) -> MeetingStoreResult<Meeting> {
        let meeting_id = meeting_id.as_str();
        let thread_id = thread_id.as_str();
        let (id, thread_id) = (meeting_id.to_owned(), thread_id.to_owned());
        self.call(move |connection| {
            write(connection, |tx| {
                records::required(
                    records::edit_meeting(tx, &id, |meeting| {
                        meeting.thread_id = Some(thread_id);
                        Ok(())
                    })?,
                    &id,
                )
            })
        })
        .await
    }

    async fn ensure_meeting_speakers(
        &self,
        meeting_id: &bruhs_models::ids::MeetingId,
        speakers: Vec<Speaker>,
    ) -> MeetingStoreResult<Meeting> {
        let meeting_id = meeting_id.as_str();
        let id = meeting_id.to_owned();
        self.call(move |connection| {
            write(connection, |tx| {
                records::required(
                    records::edit_meeting(tx, &id, |meeting| {
                        for speaker in speakers {
                            if !meeting.speakers.iter().any(|prior| prior.id == speaker.id) {
                                meeting.speakers.push(speaker);
                            }
                        }
                        Ok(())
                    })?,
                    &id,
                )
            })
        })
        .await
    }

    async fn patch_meeting_skip_observations(
        &self,
        meeting_id: &bruhs_models::ids::MeetingId,
        skip: bool,
    ) -> MeetingStoreResult<Meeting> {
        let meeting_id = meeting_id.as_str();
        let id = meeting_id.to_owned();
        self.call(move |connection| {
            write(connection, |tx| {
                records::required(
                    records::edit_meeting(tx, &id, |meeting| {
                        meeting.skip_observations = skip;
                        Ok(())
                    })?,
                    &id,
                )
            })
        })
        .await
    }

    async fn delete_meeting(&self, id: &bruhs_models::ids::MeetingId) -> MeetingStoreResult<bool> {
        let id = id.as_str();
        let id = id.to_owned();
        self.call(move |connection| {
            write(connection, |tx| {
                Ok(tx.execute("DELETE FROM meetings WHERE id=?1", [&id])? != 0)
            })
        })
        .await
    }

    async fn append_segment(&self, segment: TranscriptSegment) -> MeetingStoreResult<()> {
        self.call(move |connection| write(connection, |tx| records::append_segment(tx, &segment)))
            .await
    }

    async fn remove_segment(&self, meeting_id: &str, segment_id: &str) -> MeetingStoreResult<()> {
        let meeting_id = meeting_id.to_owned();
        let segment_id = segment_id.to_owned();
        self.call(move |connection| {
            write(connection, |tx| {
                records::remove_segment(tx, &meeting_id, &segment_id)
            })
        })
        .await
    }

    async fn segment_removed(
        &self,
        meeting_id: &str,
        segment_id: &str,
    ) -> MeetingStoreResult<bool> {
        let meeting_id = meeting_id.to_owned();
        let segment_id = segment_id.to_owned();
        self.call(move |connection| records::segment_removed(connection, &meeting_id, &segment_id))
            .await
    }

    async fn removed_segment_ids(&self, meeting_id: &str) -> MeetingStoreResult<Vec<String>> {
        let meeting_id = meeting_id.to_owned();
        self.call(move |connection| records::removed_segment_ids(connection, &meeting_id))
            .await
    }

    async fn list_segments(&self, meeting_id: &str) -> MeetingStoreResult<Vec<TranscriptSegment>> {
        let id = meeting_id.to_owned();
        self.call(move |connection| records::segments(connection, &id))
            .await
    }

    async fn cache_transcript(
        &self,
        meeting_id: &str,
        expected: Vec<TranscriptSegment>,
        incoming: Vec<TranscriptSegment>,
    ) -> MeetingStoreResult<Vec<TranscriptSegment>> {
        let id = meeting_id.to_owned();
        self.call(move |connection| {
            write(connection, |tx| {
                records::cache_transcript(tx, &id, expected, incoming)
            })
        })
        .await
    }
}

#[async_trait::async_trait]
impl PeopleStore for SqliteMeetingStore {
    async fn upsert_person(&self, person: PersonProfile) -> MeetingStoreResult<PersonProfile> {
        self.call(move |connection| write(connection, |tx| contacts::insert(tx, person)))
            .await
    }

    async fn insert_person_if_absent(
        &self,
        person: PersonProfile,
    ) -> MeetingStoreResult<crate::meetings::PersonInsert> {
        self.call(move |connection| write(connection, |tx| contacts::insert_if_absent(tx, person)))
            .await
    }

    async fn list_people(&self) -> MeetingStoreResult<Vec<PersonProfile>> {
        self.call(|connection| contacts::list(connection)).await
    }

    async fn get_person(&self, id: &str) -> MeetingStoreResult<Option<PersonProfile>> {
        let id = id.to_owned();
        self.call(move |connection| contacts::person(connection, &id))
            .await
    }

    async fn person_versions(
        &self,
        after: Option<&str>,
    ) -> MeetingStoreResult<bruhs_models::PersonVersionPage> {
        let after = after.unwrap_or_default().to_owned();
        self.call(move |connection| super::contact_sync::versions(connection, &after))
            .await
    }

    async fn get_people(&self, ids: &[String]) -> MeetingStoreResult<Vec<PersonProfile>> {
        crate::meetings::person_changes::validate_resolution_ids(ids)?;
        let ids = encode(&ids)?;
        self.call(move |connection| super::contact_sync::profiles(connection, &ids))
            .await
    }

    async fn resolve_person_merges(
        &self,
        ids: &[String],
    ) -> MeetingStoreResult<Vec<bruhs_models::PersonMergeResolution>> {
        crate::meetings::person_changes::validate_resolution_ids(ids)?;
        let ids = encode(&ids)?;
        self.call(move |connection| contacts::resolve_merges(connection, &ids))
            .await
    }

    async fn append_person_observations(
        &self,
        by_display_name: HashMap<String, Vec<PersonObservation>>,
    ) -> MeetingStoreResult<Vec<PersonProfile>> {
        self.call(move |connection| {
            write(connection, |tx| contacts::observations(tx, by_display_name))
        })
        .await
    }

    async fn match_centroid_to_person(
        &self,
        embedding: &[f32],
        threshold: f32,
    ) -> MeetingStoreResult<Option<PersonProfile>> {
        let embedding = embedding.to_vec();
        self.call(move |connection| contacts::match_voice(connection, &embedding, threshold))
            .await
    }

    async fn upsert_voice_embedding(
        &self,
        person_id: &bruhs_models::ids::PersonId,
        embedding: Vec<f32>,
    ) -> MeetingStoreResult<PersonProfile> {
        let person_id = person_id.as_str();
        let id = person_id.to_owned();
        self.call(move |connection| {
            write(connection, |tx| {
                contacts::edit(tx, &id, |person| {
                    person.voice_embedding = Some(embedding);
                    Ok(())
                })
            })
        })
        .await
    }

    async fn fill_voice_embedding(
        &self,
        person_id: &bruhs_models::ids::PersonId,
        embedding: Vec<f32>,
    ) -> MeetingStoreResult<PersonProfile> {
        let person_id = person_id.as_str();
        let id = person_id.to_owned();
        self.call(move |connection| {
            write(connection, |tx| {
                let current = contacts::required(tx, &id)?;
                if current.voice_embedding.is_some() {
                    return Ok(current);
                }
                contacts::edit(tx, &id, |person| {
                    person.voice_embedding = Some(embedding);
                    Ok(())
                })
            })
        })
        .await
    }

    async fn patch_person_tracked(
        &self,
        person_id: &bruhs_models::ids::PersonId,
        tracked: bool,
    ) -> MeetingStoreResult<PersonProfile> {
        let person_id = person_id.as_str();
        let id = person_id.to_owned();
        self.call(move |connection| {
            write(connection, |tx| {
                contacts::edit(tx, &id, |person| {
                    person.tracked = tracked;
                    Ok(())
                })
            })
        })
        .await
    }

    async fn delete_observations_for_meeting(
        &self,
        meeting_id: &bruhs_models::ids::MeetingId,
    ) -> MeetingStoreResult<usize> {
        let meeting_id = meeting_id.as_str();
        let id = meeting_id.to_owned();
        self.call(move |connection| {
            write(connection, |tx| {
                let people = contacts::list(tx)?;
                let mut deleted = 0;
                for mut person in people {
                    let prior = person.observations.len();
                    person
                        .observations
                        .retain(|observation| observation.at_meeting_id != id);
                    if person.observations.len() != prior {
                        deleted += prior - person.observations.len();
                        person.updated_at = next_version(person.updated_at)?;
                        contacts::save(tx, &person)?;
                    }
                }
                Ok(deleted)
            })
        })
        .await
    }

    async fn rename_person(
        &self,
        person_id: &bruhs_models::ids::PersonId,
        new_name: String,
    ) -> MeetingStoreResult<PersonProfile> {
        let person_id = person_id.as_str();
        let id = person_id.to_owned();
        let name = new_name.trim().to_owned();
        self.call(move |connection| {
            write(connection, |tx| {
                contacts::edit(tx, &id, |person| {
                    person.display_name = name;
                    Ok(())
                })
            })
        })
        .await
    }

    async fn merge_person_into(
        &self,
        source_id: &bruhs_models::ids::PersonId,
        target_id: &bruhs_models::ids::PersonId,
    ) -> MeetingStoreResult<PersonProfile> {
        let source_id = source_id.as_str();
        let target_id = target_id.as_str();
        let (source, target) = (source_id.to_owned(), target_id.to_owned());
        self.call(move |connection| write(connection, |tx| contacts::merge(tx, &source, &target)))
            .await
    }

    async fn link_speaker_to_person(
        &self,
        meeting_id: &bruhs_models::ids::MeetingId,
        speaker_id: &bruhs_models::ids::SpeakerId,
        person_id: &bruhs_models::ids::PersonId,
    ) -> MeetingStoreResult<Meeting> {
        let meeting_id = meeting_id.as_str();
        let speaker_id = speaker_id.as_str();
        let person_id = person_id.as_str();
        let (id, speaker_id, person_id) = (
            meeting_id.to_owned(),
            speaker_id.to_owned(),
            person_id.to_owned(),
        );
        self.call(move |connection| {
            write(connection, |tx| {
                let person = contacts::required(tx, &person_id)?;
                records::required(
                    records::edit_meeting(tx, &id, |meeting| {
                        if let Some(speaker) = meeting
                            .speakers
                            .iter_mut()
                            .find(|speaker| speaker.id == speaker_id)
                        {
                            speaker.label = person.display_name;
                            speaker.person_id = Some(person_id);
                        } else {
                            meeting.speakers.push(Speaker {
                                id: speaker_id,
                                label: person.display_name,
                                person_id: Some(person_id),
                                color: None,
                            });
                        }
                        Ok(())
                    })?,
                    &id,
                )
            })
        })
        .await
    }
}

#[async_trait::async_trait]
impl SummaryGenerationStore for SqliteMeetingStore {
    async fn pending_summary_generation(
        &self,
        meeting_id: &str,
    ) -> MeetingStoreResult<Option<bruhs_models::SummaryGenerationReceipt>> {
        let meeting = meeting_id.to_owned();
        self.call(move |connection| generation::pending(connection, &meeting))
            .await
    }

    async fn begin_summary_generation(
        &self,
        request: bruhs_models::SummaryGenerationRequest,
    ) -> MeetingStoreResult<crate::meetings::GenerationReservation> {
        self.call(move |connection| write(connection, |tx| generation::begin(tx, request)))
            .await
    }

    async fn summary_generation(
        &self,
        meeting_id: &str,
        request_id: &str,
    ) -> MeetingStoreResult<Option<bruhs_models::SummaryGenerationReceipt>> {
        let (meeting, id) = (meeting_id.to_owned(), request_id.to_owned());
        self.call(move |connection| generation::get(connection, &meeting, &id))
            .await
    }

    async fn finish_summary_generation(
        &self,
        meeting_id: &str,
        request_id: &str,
        outcome: crate::meetings::GenerationOutcome,
    ) -> MeetingStoreResult<bruhs_models::SummaryGenerationReceipt> {
        let (meeting, id) = (meeting_id.to_owned(), request_id.to_owned());
        self.call(move |connection| {
            write(connection, |tx| {
                generation::finish(tx, &meeting, &id, outcome)
            })
        })
        .await
    }

    async fn fence_summary_generation(
        &self,
        request: bruhs_models::SummaryGenerationRequest,
    ) -> MeetingStoreResult<bruhs_models::SummaryGenerationReceipt> {
        self.call(move |connection| write(connection, |tx| generation::fence(tx, request)))
            .await
    }

    async fn commit_summary_generation(
        &self,
        request_id: &str,
        update: crate::meetings::SummaryUpdate,
    ) -> MeetingStoreResult<Meeting> {
        let id = request_id.to_owned();
        self.call(move |connection| write(connection, |tx| generation::commit(tx, &id, update)))
            .await
    }

    async fn summary_people_versions(
        &self,
        expected: &Meeting,
    ) -> MeetingStoreResult<std::collections::BTreeMap<String, i64>> {
        let expected = expected.clone();
        self.call(move |connection| {
            write(connection, |tx| {
                summaries::participant_versions(tx, &expected)
            })
        })
        .await
    }

    async fn commit_summary(
        &self,
        update: crate::meetings::SummaryUpdate,
    ) -> MeetingStoreResult<Meeting> {
        self.call(move |connection| write(connection, |tx| summaries::commit(tx, update)))
            .await
    }
}

#[async_trait::async_trait]
impl SpeakerLinkJobs for SqliteMeetingStore {
    async fn enqueue_speaker_link(&self, meeting_id: &str) -> MeetingStoreResult<()> {
        let job = crate::meetings::SpeakerLinkJob::new(meeting_id);
        self.call(move |connection| write(connection, |tx| link_jobs::enqueue(tx, &job)))
            .await
    }

    async fn enqueue_capture_receipt(
        &self,
        receipt: &bruhs_models::CaptureRecoveryReceipt,
    ) -> MeetingStoreResult<()> {
        let job = crate::meetings::SpeakerLinkJob::from_capture(receipt)?;
        self.call(move |connection| write(connection, |tx| link_jobs::enqueue(tx, &job)))
            .await
    }

    async fn next_speaker_link(
        &self,
        now_ms: i64,
    ) -> MeetingStoreResult<Option<crate::meetings::SpeakerLinkJob>> {
        self.call(move |connection| link_jobs::next(connection, now_ms))
            .await
    }

    async fn finish_speaker_link(
        &self,
        job: &crate::meetings::SpeakerLinkJob,
    ) -> MeetingStoreResult<()> {
        let job = job.clone();
        self.call(move |connection| write(connection, |tx| link_jobs::finish(tx, &job)))
            .await
    }

    async fn retry_speaker_link(
        &self,
        job: &crate::meetings::SpeakerLinkJob,
        now_ms: i64,
    ) -> MeetingStoreResult<()> {
        let job = job.clone();
        self.call(move |connection| write(connection, |tx| link_jobs::retry(tx, &job, now_ms)))
            .await
    }

    async fn link_speaker_centroids(
        &self,
        meeting_id: &str,
        centroids: &bruhs_models::CentroidsFile,
    ) -> MeetingStoreResult<bool> {
        let id = meeting_id.to_owned();
        let centroids = centroids.clone();
        self.call(move |connection| {
            write(connection, |tx| {
                let meeting = records::required(records::meeting(tx, &id)?, &id)?;
                if !crate::meetings::speaker_linking::needs_linking(&meeting, &centroids) {
                    return Ok(false);
                }
                let people = contacts::list(tx)?
                    .into_iter()
                    .map(|person| (person.id.clone(), person))
                    .collect();
                let Some(update) =
                    crate::meetings::speaker_linking::plan(meeting, &people, &centroids)?
                else {
                    return Ok(false);
                };
                for person in update.people.values() {
                    contacts::save(tx, person)?;
                }
                records::save_meeting(tx, &update.meeting)?;
                Ok(true)
            })
        })
        .await
    }
}
