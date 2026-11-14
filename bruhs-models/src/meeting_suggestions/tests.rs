use super::*;

fn segment(id: &str, speaker: &str, start_ms: u32, end_ms: u32, text: &str) -> TranscriptSegment {
    TranscriptSegment {
        id: id.into(),
        meeting_id: "m1".into(),
        speaker_id: Some(speaker.into()),
        start_ms,
        end_ms,
        text: text.into(),
        confidence: TranscriptSegment::UNKNOWN_CONFIDENCE,
        partial: true,
        speaker_confidence: None,
    }
}

fn prefs(proactivity: Proactivity) -> MeetingSuggestionPrefs {
    MeetingSuggestionPrefs {
        proactivity,
        ..MeetingSuggestionPrefs::default()
    }
}

fn input<'a>(
    now: f64,
    prefs: &'a MeetingSuggestionPrefs,
    segments: &'a [TranscriptSegment],
) -> LiveInput<'a> {
    LiveInput {
        now,
        prefs,
        meeting_title: "Launch sync",
        segments,
        speakers: &[],
    }
}

fn thought(kind: ThoughtKind, text: &str, score: u8, source: Option<&str>) -> Thought {
    Thought {
        kind,
        text: text.into(),
        reason_for: "It was just agreed.".into(),
        reason_against: "It may already be tracked.".into(),
        score,
        source: source.map(str::to_string),
    }
}

const LONG: &str = "Dana will send the revised launch deck to the whole team by Thursday afternoon, and we still need someone to confirm the venue booking for the offsite next week. Priya owns the budget review.";

#[test]
fn default_prefs_only_answer_when_asked_and_survive_storage() {
    let saved = MeetingSuggestionPrefs::default();
    assert_eq!(saved.proactivity, Proactivity::OnRequest);
    assert_eq!(saved.model(), DEFAULT_MODEL);
    assert_eq!(saved.calibration.threshold, DEFAULT_THRESHOLD);
    let restored: MeetingSuggestionPrefs = serde_json::from_str("{}").unwrap();
    assert_eq!(restored, saved);
    let off = MeetingSuggestionPrefs {
        proactivity: Proactivity::Off,
        ..saved
    };
    let json = serde_json::to_string(&off).unwrap();
    assert!(json.contains("\"off\""), "{json}");
    assert_eq!(
        serde_json::from_str::<MeetingSuggestionPrefs>(&json).unwrap(),
        off
    );
    for level in Proactivity::ALL {
        assert_eq!(Proactivity::parse(level.as_str()), Some(level));
    }
}

#[test]
fn only_isolated_runtimes_are_kept_as_the_model() {
    let prefs = MeetingSuggestionPrefs {
        model: Some("cursor/sonnet-4".into()),
        calibration: Calibration {
            threshold: 9,
            outcomes: vec![true; 40],
        },
        ..MeetingSuggestionPrefs::default()
    }
    .sanitized();
    assert_eq!(prefs.model, None);
    assert_eq!(prefs.calibration.threshold, MAX_THRESHOLD);
    assert_eq!(prefs.calibration.outcomes.len(), 12);
    let codex = MeetingSuggestionPrefs {
        model: Some("gpt-6-luna".into()),
        ..MeetingSuggestionPrefs::default()
    }
    .sanitized();
    assert_eq!(codex.model(), "gpt-6-luna");
}

#[test]
fn dismissals_raise_the_bar_and_kept_suggestions_lower_it() {
    let mut calibration = Calibration::default();
    for _ in 0..5 {
        assert!(!calibration.record(false));
    }
    assert!(calibration.record(false), "six dismissals raise the bar");
    assert_eq!(calibration.threshold, 5);
    for _ in 0..6 {
        calibration.record(false);
    }
    assert_eq!(
        calibration.threshold, MAX_THRESHOLD,
        "never above the maximum"
    );
    let mut calibration = Calibration::default();
    for _ in 0..6 {
        calibration.record(true);
    }
    assert_eq!(calibration.threshold, 3);
    for _ in 0..12 {
        calibration.record(true);
    }
    assert_eq!(
        calibration.threshold, MIN_THRESHOLD,
        "never below the minimum"
    );
    let mut mixed = Calibration::default();
    for kept in [true, false, true, false, true, false, true, false] {
        mixed.record(kept);
    }
    assert_eq!(mixed.threshold, DEFAULT_THRESHOLD, "a mixed record holds");
}

#[test]
fn the_window_keeps_the_newest_two_minutes_merged_by_speaker() {
    let segments = vec![
        segment("old", "me", 0, 4_000, "Long ago"),
        segment("a", "me", 200_000, 205_000, "Let's  ship\nFriday."),
        segment("b", "me", 205_000, 210_000, "Agreed?"),
        segment("c", "them", 210_000, 215_000, "Yes, Friday."),
        segment("blank", "them", 215_000, 216_000, "   "),
    ];
    let lines = transcript_window(&segments, &[]);
    assert_eq!(
        lines,
        vec![
            TranscriptLine {
                speaker: "Me".into(),
                text: "Let's ship Friday. Agreed?".into()
            },
            TranscriptLine {
                speaker: "Others".into(),
                text: "Yes, Friday.".into()
            },
        ]
    );
    let mut speaker = Speaker::new("Dana");
    speaker.id = "them".into();
    assert_eq!(transcript_window(&segments, &[speaker])[1].speaker, "Dana");
}

#[test]
fn the_window_fits_the_request_budget_from_the_newest_end() {
    let segments: Vec<_> = (0..200u32)
        .map(|index| {
            let speaker = if index % 2 == 0 { "me" } else { "them" };
            segment(
                &index.to_string(),
                speaker,
                index * 500,
                index * 500 + 400,
                &"word ".repeat(40),
            )
        })
        .collect();
    let lines = transcript_window(&segments, &[]);
    let chars: usize = lines.iter().map(|line| line.text.chars().count()).sum();
    assert!(lines.len() <= MAX_TRANSCRIPT_LINES);
    assert!(chars <= MAX_TRANSCRIPT_CHARS);
    assert!(lines
        .iter()
        .all(|line| line.text.chars().count() <= MAX_LINE_CHARS));
}

#[test]
fn retrieval_keeps_the_meeting_and_what_the_talk_touches() {
    let transcript = vec![TranscriptLine {
        speaker: "Others".into(),
        text: "Can you send the launch decks before the offsite?".into(),
    }];
    let selected = select_context(
        vec![
            ContextItem::task("task-1", "Groceries", None),
            ContextItem::task("task-2", "Send launch deck to Dana", Some("2026-11-20")),
            ContextItem::event("event-1", "Team offsite", "Tue 09:00", &["Dana".into()]),
            ContextItem::meeting("meeting-1", "Launch sync", &[]),
            ContextItem::person("person-1", "Priya"),
        ],
        &transcript,
    );
    let references: Vec<_> = selected
        .iter()
        .map(|item| (item.reference.as_str(), item.id.as_str()))
        .collect();
    assert_eq!(
        references,
        [("m", "meeting-1"), ("t1", "task-2"), ("e1", "event-1")]
    );
}

#[test]
fn retrieval_never_exceeds_the_budget() {
    let transcript = vec![TranscriptLine {
        speaker: "Me".into(),
        text: "budget review".into(),
    }];
    let items = (0..30)
        .map(|index| ContextItem::task(format!("task-{index}"), "Budget review", None))
        .collect();
    let selected = select_context(items, &transcript);
    assert_eq!(selected.len(), MAX_CONTEXT_ITEMS);
    assert_eq!(selected.last().unwrap().reference, "t8");
}

#[test]
fn record_ids_never_reach_the_model() {
    let item = ContextItem::task("secret-record-id", "Pay invoice", None);
    let json = serde_json::to_string(&item).unwrap();
    assert!(!json.contains("secret-record-id"), "{json}");
}

#[test]
fn replies_are_bounded_and_structured() {
    let valid = MeetingSuggestionReply {
        thoughts: vec![thought(
            ThoughtKind::ActionItem,
            "Dana sends the deck",
            4,
            Some("t1"),
        )],
    };
    assert!(valid.is_valid());
    let json = serde_json::to_string(&valid).unwrap();
    assert!(
        json.find("\"for\"").unwrap() < json.find("\"score\"").unwrap(),
        "reasons come before the score: {json}"
    );
    for bad in [
        MeetingSuggestionReply {
            thoughts: vec![thought(ThoughtKind::Question, "x", 6, None)],
        },
        MeetingSuggestionReply {
            thoughts: vec![thought(ThoughtKind::Question, "two\nlines", 4, None)],
        },
        MeetingSuggestionReply {
            thoughts: vec![thought(ThoughtKind::Question, "", 4, None)],
        },
        MeetingSuggestionReply {
            thoughts: vec![thought(ThoughtKind::Question, "ok", 4, Some("../etc"))],
        },
        MeetingSuggestionReply {
            thoughts: vec![thought(ThoughtKind::Question, "ok", 4, None); 4],
        },
    ] {
        assert!(!bad.is_valid(), "{bad:?}");
    }
    assert!(
        serde_json::from_str::<MeetingSuggestionReply>(r#"{"thoughts":[],"note":"extra"}"#)
            .is_err()
    );
}

#[test]
fn automatic_rounds_follow_speech_and_pauses_and_never_overlap() {
    let auto = prefs(Proactivity::Automatic);
    let mut live = LiveSuggestions::default();
    live.follow(Some("m1"));
    let short = vec![segment(
        "a",
        "me",
        0,
        5_000,
        "Hello there everyone, thanks for joining the sync.",
    )];
    assert!(live.poll(&input(0.0, &auto, &short), Vec::new).is_none());
    // A pause after a little speech starts a round.
    assert!(live
        .poll(&input(9_000.0, &auto, &short), Vec::new)
        .is_none());
    let first = live
        .poll(&input(10_500.0, &auto, &short), Vec::new)
        .expect("pause round");
    assert_eq!(first.request.trigger, Trigger::Pause);
    assert!(live.is_thinking());
    let long = vec![
        segment(
            "a",
            "me",
            0,
            5_000,
            "Hello there everyone, thanks for joining the sync.",
        ),
        segment("b", "them", 5_000, 10_000, LONG),
    ];
    assert!(
        live.poll(&input(12_000.0, &auto, &long), Vec::new)
            .is_none(),
        "one round at a time"
    );
    live.finish(
        &first,
        MeetingSuggestionReply::default(),
        4,
        13_000.0,
        || "x".into(),
    );
    assert!(
        live.poll(&input(20_000.0, &auto, &long), Vec::new)
            .is_none(),
        "rounds are spaced"
    );
    let second = live
        .poll(&input(41_000.0, &auto, &long), Vec::new)
        .expect("speech round");
    assert_eq!(second.request.trigger, Trigger::Speech);
    live.finish(
        &second,
        MeetingSuggestionReply::default(),
        4,
        42_000.0,
        || "x".into(),
    );
    assert!(
        live.poll(&input(200_000.0, &auto, &long), Vec::new)
            .is_none(),
        "covered speech does not trigger again"
    );
}

#[test]
fn on_request_and_paused_sessions_never_start_automatically() {
    let segments = vec![segment("b", "them", 0, 10_000, LONG)];
    for (level, paused) in [
        (Proactivity::OnRequest, false),
        (Proactivity::Off, false),
        (Proactivity::Alert, true),
    ] {
        let prefs = prefs(level);
        let mut live = LiveSuggestions::default();
        live.follow(Some("m1"));
        live.paused = paused;
        for now in [0.0, 60_000.0, 120_000.0] {
            assert!(live
                .poll(&input(now, &prefs, &segments), Vec::new)
                .is_none());
        }
    }
    let on_request = prefs(Proactivity::OnRequest);
    let mut live = LiveSuggestions::default();
    live.follow(Some("m1"));
    let round = live
        .request(&input(1.0, &on_request, &segments), Vec::new())
        .expect("Suggest now");
    assert_eq!(round.request.trigger, Trigger::Request);
    assert_eq!(round.request.model, DEFAULT_MODEL);
    assert!(live
        .request(&input(2.0, &on_request, &segments), Vec::new())
        .is_none());
    let off = prefs(Proactivity::Off);
    let mut live = LiveSuggestions::default();
    live.follow(Some("m1"));
    assert!(live
        .request(&input(1.0, &off, &segments), Vec::new())
        .is_none());
}

#[test]
fn suggest_now_explains_an_empty_transcript() {
    let prefs = prefs(Proactivity::OnRequest);
    let mut live = LiveSuggestions::default();
    live.follow(Some("m1"));
    assert!(live.request(&input(1.0, &prefs, &[]), Vec::new()).is_none());
    assert!(matches!(live.status, RoundStatus::Failed(_)));
}

#[test]
fn replies_are_filtered_by_threshold_and_repeats_and_map_sources_back() {
    let prefs = prefs(Proactivity::OnRequest);
    let segments = vec![segment("b", "them", 0, 10_000, LONG)];
    let mut live = LiveSuggestions::default();
    live.follow(Some("m1"));
    let round = live
        .request(
            &input(1.0, &prefs, &segments),
            vec![
                ContextItem::meeting("meeting-1", "Launch sync", &[]),
                ContextItem::task("task-9", "Revised launch deck", None),
            ],
        )
        .unwrap();
    assert_eq!(round.request.context[1].reference, "t1");
    let mut ids = 0;
    let added = live.finish(
        &round,
        MeetingSuggestionReply {
            thoughts: vec![
                thought(ThoughtKind::Context, "Low value aside", 2, None),
                thought(
                    ThoughtKind::ActionItem,
                    "Dana sends the revised launch deck by Thursday",
                    5,
                    Some("t1"),
                ),
                thought(
                    ThoughtKind::Question,
                    "Who confirms the venue booking?",
                    4,
                    Some("m"),
                ),
            ],
        },
        4,
        2.0,
        || {
            ids += 1;
            format!("s{ids}")
        },
    );
    assert_eq!(added, 2);
    assert_eq!(live.items[0].kind, ThoughtKind::Question, "newest first");
    assert_eq!(
        live.items[0].source, None,
        "the meeting itself is not a source"
    );
    assert_eq!(
        live.items[1].source,
        Some((ContextKind::Task, "task-9".to_string()))
    );
    assert_eq!(live.unseen(), 2);
    // The same idea again is not shown twice, and repeats are sent back.
    let again = live
        .request(&input(3.0, &prefs, &segments), Vec::new())
        .unwrap();
    assert_eq!(again.request.recent.len(), 2);
    let added = live.finish(
        &again,
        MeetingSuggestionReply {
            thoughts: vec![thought(
                ThoughtKind::ActionItem,
                "Dana sends revised launch deck by Thursday",
                5,
                None,
            )],
        },
        4,
        4.0,
        || "dup".into(),
    );
    assert_eq!(added, 0);
}

#[test]
fn a_late_reply_for_another_meeting_is_ignored() {
    let prefs = prefs(Proactivity::OnRequest);
    let segments = vec![segment("b", "them", 0, 10_000, LONG)];
    let mut live = LiveSuggestions::default();
    live.follow(Some("m1"));
    let round = live
        .request(&input(1.0, &prefs, &segments), Vec::new())
        .unwrap();
    live.follow(Some("m2"));
    let added = live.finish(
        &round,
        MeetingSuggestionReply {
            thoughts: vec![thought(ThoughtKind::Question, "Stale", 5, None)],
        },
        3,
        2.0,
        || "s".into(),
    );
    assert_eq!(added, 0);
    assert!(live.items.is_empty());
    assert!(!live.is_thinking());
}

#[test]
fn failures_back_off_before_the_next_automatic_round() {
    let auto = prefs(Proactivity::Automatic);
    let segments = vec![segment("b", "them", 0, 10_000, LONG)];
    let mut live = LiveSuggestions::default();
    live.follow(Some("m1"));
    let round = live.poll(&input(0.0, &auto, &segments), Vec::new).unwrap();
    live.fail(&round, 1_000.0, Some(502));
    assert_eq!(
        live.status,
        RoundStatus::Failed(failure_message(Some(502)).to_string())
    );
    assert!(live
        .poll(&input(40_000.0, &auto, &segments), Vec::new)
        .is_none());
    let retry = live
        .poll(&input(62_000.0, &auto, &segments), Vec::new)
        .unwrap();
    // A busy server (another app holding the slot) is retried sooner.
    live.fail(&retry, 63_000.0, Some(503));
    assert!(live
        .poll(&input(70_000.0, &auto, &segments), Vec::new)
        .is_none());
    assert!(live
        .poll(&input(79_000.0, &auto, &segments), Vec::new)
        .is_some());
}

#[test]
fn decisions_are_final_and_feed_calibration() {
    let prefs = prefs(Proactivity::OnRequest);
    let segments = vec![segment("b", "them", 0, 10_000, LONG)];
    let mut live = LiveSuggestions::default();
    live.follow(Some("m1"));
    let round = live
        .request(&input(1.0, &prefs, &segments), Vec::new())
        .unwrap();
    live.finish(
        &round,
        MeetingSuggestionReply {
            thoughts: vec![thought(ThoughtKind::ActionItem, "Book the venue", 5, None)],
        },
        4,
        2.0,
        || "s1".into(),
    );
    assert!(live.mark_seen());
    assert_eq!(live.unseen(), 0);
    let copied = live.decide("s1", SuggestionState::Copied).unwrap();
    assert_eq!(copied.kept, Some(true), "the first decision counts");
    let again = live.decide("s1", SuggestionState::Copied).unwrap();
    assert_eq!(again.suggestion.text, "Book the venue", "Copy may repeat");
    assert_eq!(again.kept, None, "but counts once");
    let added = live.decide("s1", SuggestionState::Added).unwrap();
    assert_eq!(added.kept, None, "adding after a copy is not a second keep");
    assert_eq!(added.suggestion.state, SuggestionState::Added);
    assert!(live.decide("s1", SuggestionState::Dismissed).is_none());
    assert!(live.decide("s1", SuggestionState::Added).is_none());
    assert!(live.decide("s1", SuggestionState::Copied).is_some());
    assert!(live.decide("missing", SuggestionState::Copied).is_none());
}

#[test]
fn a_dismissal_counts_once_and_is_final() {
    let prefs = prefs(Proactivity::OnRequest);
    let segments = vec![segment("b", "them", 0, 10_000, LONG)];
    let mut live = LiveSuggestions::default();
    live.follow(Some("m1"));
    let round = live
        .request(&input(1.0, &prefs, &segments), Vec::new())
        .unwrap();
    live.finish(
        &round,
        MeetingSuggestionReply {
            thoughts: vec![thought(
                ThoughtKind::Question,
                "Who books the venue?",
                5,
                None,
            )],
        },
        4,
        2.0,
        || "s1".into(),
    );
    assert_eq!(
        live.decide("s1", SuggestionState::Dismissed).unwrap().kept,
        Some(false)
    );
    assert!(live.decide("s1", SuggestionState::Copied).is_none());
    assert!(live.decide("s1", SuggestionState::Added).is_none());
}

#[test]
fn a_new_meeting_eases_a_raised_bar() {
    let mut calibration = Calibration {
        threshold: MAX_THRESHOLD,
        outcomes: vec![false; 3],
    };
    assert!(calibration.relax());
    assert_eq!(calibration.threshold, DEFAULT_THRESHOLD);
    assert!(calibration.outcomes.is_empty());
    assert!(!calibration.relax(), "never below the default");
    let mut low = Calibration {
        threshold: MIN_THRESHOLD,
        outcomes: Vec::new(),
    };
    assert!(!low.relax(), "a lowered bar stays low");
}

#[test]
fn context_items_are_single_line_and_bounded() {
    let item = ContextItem::event(
        "e",
        &"Planning\nreview ".repeat(40),
        "Mon 10:00",
        &["Ana".into(), "Bo".into()],
    );
    assert!(!item.text.contains('\n'));
    assert!(item.text.chars().count() <= MAX_CONTEXT_CHARS);
    assert!(event_in_window(1_000.0, 0.0));
    assert!(!event_in_window(8.0 * 86_400_000.0, 0.0));
}

#[test]
fn a_cancelled_round_keeps_suggestions_and_ignores_its_reply() {
    let prefs = prefs(Proactivity::OnRequest);
    let segments = vec![segment("b", "them", 0, 10_000, LONG)];
    let mut live = LiveSuggestions::default();
    live.follow(Some("m1"));
    let first = live
        .request(&input(1.0, &prefs, &segments), Vec::new())
        .unwrap();
    live.finish(
        &first,
        MeetingSuggestionReply {
            thoughts: vec![thought(
                ThoughtKind::Question,
                "Who books the venue?",
                5,
                None,
            )],
        },
        4,
        2.0,
        || "s1".into(),
    );
    let second = live
        .request(&input(3.0, &prefs, &segments), Vec::new())
        .unwrap();
    live.cancel();
    assert!(!live.is_thinking());
    assert_eq!(live.items.len(), 1);
    let late = MeetingSuggestionReply {
        thoughts: vec![thought(ThoughtKind::Conflict, "Late idea", 5, None)],
    };
    assert_eq!(live.finish(&second, late, 3, 4.0, || "s2".into()), 0);
    assert!(live
        .request(&input(5.0, &prefs, &segments), Vec::new())
        .is_some());
}

#[test]
fn failures_read_the_same_on_both_apps() {
    assert!(failure_message(None).contains("reach the server"));
    assert!(failure_message(Some(503)).contains("usage"));
    assert_ne!(failure_message(Some(500)), failure_message(Some(504)));
}

#[test]
fn rephrased_notes_are_repeats_and_related_notes_are_not() {
    assert!(
        similarity(
            "Should the free tier be kept for this release?",
            "Decide: keep the free tier for this release or not?"
        ) >= DUPLICATE_SIMILARITY
    );
    assert!(
        similarity(
            "Dana sends the revised launch deck by Thursday",
            "Who reviews the launch deck before the board?"
        ) < DUPLICATE_SIMILARITY
    );
    assert!(similarity("Book the venue", "Pay the venue invoice") < DUPLICATE_SIMILARITY);
}
