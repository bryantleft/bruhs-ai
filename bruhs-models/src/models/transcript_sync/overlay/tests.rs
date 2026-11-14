use super::*;
use crate::{validate_transcript_records, MAX_TRANSCRIPT_SEGMENTS};

fn row(id: &str, partial: bool) -> TranscriptSegment {
    TranscriptSegment {
        id: id.into(),
        meeting_id: "meeting".into(),
        speaker_id: Some("speaker".into()),
        start_ms: 10,
        end_ms: 20,
        text: "Saved ΟΣ Café 👩🏽‍💻 \"words\"\n".into(),
        confidence: 0.9,
        partial,
        speaker_confidence: Some(0.8),
    }
}

// Original native validation algorithm, including its owned merge and ordering.
fn original(
    meeting_id: &str,
    base: &[TranscriptSegment],
    fragments: &[TranscriptSegment],
) -> Result<(), &'static str> {
    validate_transcript_records(meeting_id, fragments)
        .map_err(TranscriptValidationError::message)?;
    let existing: HashMap<_, _> = base.iter().map(|r| (r.id.as_str(), r)).collect();
    if fragments.iter().any(|r| {
        existing
            .get(r.id.as_str())
            .is_some_and(|saved| !saved.partial && *saved != r)
    }) {
        return Err("Saved stream fragments cannot rewrite finalized speech.");
    }
    let mut merged = base.to_vec();
    let mut positions: HashMap<_, _> = merged
        .iter()
        .enumerate()
        .map(|(i, r)| (r.id.clone(), i))
        .collect();
    for r in fragments {
        if let Some(i) = positions.get(&r.id) {
            merged[*i] = r.clone();
        } else {
            positions.insert(r.id.clone(), merged.len());
            merged.push(r.clone());
        }
    }
    merged.sort_unstable_by(|a, b| (a.start_ms, &a.id).cmp(&(b.start_ms, &b.id)));
    validate_transcript_records(meeting_id, &merged).map_err(TranscriptValidationError::message)
}

fn check(
    meeting_id: &str,
    base: &[TranscriptSegment],
    fragments: &[TranscriptSegment],
    expected: Result<(), &'static str>,
) {
    let before = (base.len() + fragments.len() < 512).then(|| format!("{base:?}{fragments:?}"));
    let borrowed = fragments.iter().collect::<Vec<_>>();
    assert_eq!(original(meeting_id, base, fragments), expected);
    assert_eq!(
        validate_transcript_overlay(meeting_id, base, &borrowed),
        expected
    );
    // Debug preserves NaN; input equality alone cannot check those refusal cases.
    if let Some(before) = before {
        assert_eq!(format!("{base:?}{fragments:?}"), before);
    }
}

#[test]
fn overlays_preserve_unsorted_inputs_and_allow_partial_replacement_and_append() {
    let base = [row("z", false), row("a", true), row("same", false)];
    let mut changed = row("a", false);
    changed.text = "Final words".into();
    changed.start_ms = 0;
    let mut appended = row("new", true);
    appended.start_ms = 1;
    check(
        "meeting",
        &base,
        &[changed, appended, row("same", false)],
        Ok(()),
    );
    check("meeting", &[], &[], Ok(()));
    check(
        "",
        &[],
        &[],
        Err(TranscriptValidationError::TooLarge.message()),
    );
}

#[test]
fn overlays_reject_every_finalized_field_change_with_the_original_refusal() {
    let saved = row("saved", false);
    let changes: [fn(&mut TranscriptSegment); 8] = [
        |r| r.text.push('!'),
        |r| r.start_ms += 1,
        |r| r.end_ms += 1,
        |r| r.confidence = 0.1,
        |r| r.partial = true,
        |r| r.speaker_id = Some("other".into()),
        |r| r.speaker_confidence = Some(0.1),
        |r| r.speaker_confidence = None,
    ];
    for change in changes {
        let mut incoming = saved.clone();
        change(&mut incoming);
        check(
            "meeting",
            std::slice::from_ref(&saved),
            &[incoming],
            Err("Saved stream fragments cannot rewrite finalized speech."),
        );
    }
}

#[test]
fn overlays_validate_off_page_fragments_and_the_entire_merged_view() {
    let base = (0..130)
        .map(|i| row(&format!("b{i}"), true))
        .collect::<Vec<_>>();
    let fragments = (0..130)
        .map(|i| row(&format!("f{i}"), true))
        .collect::<Vec<_>>();
    let changes: [fn(&mut TranscriptSegment); 12] = [
        |r| r.id.clear(),
        |r| r.id = "x".repeat(1025),
        |r| r.id = "f0".into(),
        |r| r.meeting_id = "foreign".into(),
        |r| r.meeting_id = "x".repeat(1025),
        |r| r.speaker_id = Some(" \n".into()),
        |r| r.speaker_id = Some("x".repeat(1025)),
        |r| r.end_ms = 0,
        |r| r.confidence = f32::NAN,
        |r| r.speaker_confidence = Some(f32::INFINITY),
        |r| r.text = "x".repeat(64 * 1024 + 1),
        |r| r.id = "\t\n".into(),
    ];
    for change in changes {
        let mut changed = fragments.clone();
        change(&mut changed[129]);
        check(
            "meeting",
            &base,
            &changed,
            Err(TranscriptValidationError::InvalidSegments.message()),
        );
    }
    let mut invalid_base = base.clone();
    invalid_base[129].confidence = f32::INFINITY;
    check(
        "meeting",
        &invalid_base,
        &fragments,
        Err(TranscriptValidationError::InvalidSegments.message()),
    );
    // The original merge can repair an invalid partial row before validating it.
    check("meeting", &invalid_base, &[base[129].clone()], Ok(()));
    let duplicate = [row("same", true), row("same", true)];
    check(
        "meeting",
        &duplicate,
        &[row("same", true)],
        Err(TranscriptValidationError::InvalidSegments.message()),
    );
    let mut invalid_fragment = row("b0", true);
    invalid_fragment.confidence = f32::NAN;
    let finalized = [row("b0", false)];
    check(
        "meeting",
        &finalized,
        &[invalid_fragment],
        Err(TranscriptValidationError::InvalidSegments.message()),
    );
}

#[test]
fn overlays_enforce_fragment_and_merged_record_and_text_limits() {
    let base = (0..MAX_TRANSCRIPT_SEGMENTS)
        .map(|i| {
            let mut r = row(&format!("b{i}"), true);
            r.text.clear();
            r
        })
        .collect::<Vec<_>>();
    let too_large = Err(TranscriptValidationError::TooLarge.message());
    check("meeting", &base, &[row("extra", true)], too_large);
    let mut too_many = base;
    too_many.push(row("extra", true));
    check("meeting", &[], &too_many, too_large);
    let mut text = (0..1024)
        .map(|i| {
            let mut r = row(&format!("t{i}"), true);
            r.text = "x".repeat(64 * 1024);
            r
        })
        .collect::<Vec<_>>();
    // Each row is valid; the extra fragment alone takes the merged text over64MiB.
    check(
        "meeting",
        &text,
        &[row("extra", true)],
        Err(TranscriptValidationError::InvalidSegments.message()),
    );
    text.push(row("extra", true));
    check(
        "meeting",
        &[],
        &text,
        Err(TranscriptValidationError::InvalidSegments.message()),
    );
}
