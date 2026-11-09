use super::*;

fn fixture() -> (Owner, TranscriptSync, Meeting) {
    let owner = Owner::new();
    let (sync, meeting) = owner.with(|| {
        let mut meeting = Meeting::new("Readback", 10.0);
        meeting.id = "meeting".into();
        let (meetings, set_meetings) = signal(vec![meeting.clone()]);
        let (segments, set_segments) = signal(BTreeMap::new());
        let (live, set_live) = signal(Some(meeting.id.clone()));
        (
            TranscriptSync::new(
                meetings,
                set_meetings,
                segments,
                set_segments,
                live,
                set_live,
                crate::components::banner::BannerStackSignal(RwSignal::new(Default::default())),
            ),
            meeting,
        )
    });
    (owner, sync, meeting)
}

#[test]
fn http_readback_rejects_new_speech_even_after_storage_commit() {
    let (_owner, sync, meeting) = fixture();
    let mut receipt = None;
    sync.stream
        .update_value(|buffer| receipt = Some(buffer.receipt(&meeting.id)));
    let receipt = receipt.unwrap();
    assert!(sync
        .check_readback(&meeting.id, &receipt, &Some(meeting.clone()))
        .is_ok());
    // The same guard runs before storage and again after terminal completion.
    sync.stream
        .update_value(|buffer| buffer.replaced(&meeting.id));
    assert!(sync
        .check_readback(&meeting.id, &receipt, &Some(meeting.clone()))
        .is_err());
}

#[test]
fn same_version_local_edit_fences_late_http_metadata() {
    let (_owner, sync, meeting) = fixture();
    let mut receipt = None;
    sync.stream
        .update_value(|buffer| receipt = Some(buffer.receipt(&meeting.id)));
    sync.set_meetings
        .update(|all| all[0].title = "Unsaved title".into());
    assert!(sync
        .check_readback(&meeting.id, &receipt.unwrap(), &Some(meeting.clone()))
        .is_err());
    assert_eq!(sync.meetings.get_untracked()[0].title, "Unsaved title");
}

#[test]
fn ended_readback_clears_only_the_matching_live_identity() {
    let (_owner, sync, mut meeting) = fixture();
    meeting.status = crate::state::MeetingStatus::Ended;
    meeting.updated_at += 1;
    sync.publish_meeting(meeting.clone());
    assert!(sync.live.get_untracked().is_none());
    sync.set_live.set(Some("another-meeting".into()));
    sync.publish_meeting(meeting);
    assert_eq!(
        sync.live.get_untracked().as_deref(),
        Some("another-meeting")
    );
}

#[test]
fn stale_ended_metadata_cannot_stop_a_newer_live_context() {
    let (_owner, sync, mut meeting) = fixture();
    meeting.status = crate::state::MeetingStatus::Ended;
    meeting.updated_at -= 1;
    sync.publish_meeting(meeting);
    assert_eq!(sync.live.get_untracked().as_deref(), Some("meeting"));
    assert_eq!(
        sync.meetings.get_untracked()[0].status,
        crate::state::MeetingStatus::Live
    );
}
