//! JSONL log of `TranscriptSegment`s alongside each meeting's WAV file.
//! One JSON object per line, terminated by `\n`. We use append-only
//! semantics so partial transcripts survive crashes.

use bruhs_models::TranscriptSegment;
use std::io::{Read, Write};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum LogError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),
}

pub fn append_segment(jsonl_path: &Path, segment: &TranscriptSegment) -> Result<(), LogError> {
    let mut file = crate::paths::append_private_file(jsonl_path)?;
    let line = serde_json::to_string(segment)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}

// Tolerant manual salvage only: synchronization and automatic finalization must
// use the strict reader so corruption cannot silently shrink a transcript.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "kept for manual salvage; only tests call it today"
    )
)]
pub fn load_segments(jsonl_path: &Path) -> Result<Vec<TranscriptSegment>, LogError> {
    if !jsonl_path.exists() {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(jsonl_path)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|e| LogError::Io(std::io::Error::other(format!("non-utf8 jsonl: {e}"))))?;
    let mut out = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<TranscriptSegment>(line) {
            Ok(seg) => out.push(seg),
            Err(e) => {
                tracing::warn!("skipping malformed jsonl line {idx}: {e}");
            }
        }
    }
    Ok(out)
}

/// Strict, bounded read for synchronization. A damaged line is not a smaller
/// successful transcript; tolerant recovery remains available via `load_segments`.
pub fn load_snapshot(jsonl_path: &Path) -> Result<Vec<TranscriptSegment>, LogError> {
    let file = match std::fs::File::open(jsonl_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    const LIMIT: u64 = 64 * 1024 * 1024;
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err(std::io::Error::other("Transcript exceeds snapshot limit").into());
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| std::io::Error::other("Transcript is not UTF-8"))?;
    let mut segments = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        if segments.len() == 100_000 {
            return Err(std::io::Error::other("Transcript exceeds record limit").into());
        }
        segments.push(serde_json::from_str(line)?);
    }
    Ok(segments)
}

/// Replace the jsonl file with `segments`, one per line. Used by
/// diarize to write speaker_id back after transcription completes.
pub fn write_all(jsonl_path: &Path, segments: &[TranscriptSegment]) -> Result<(), LogError> {
    // Post-diarization failure must not truncate the only complete transcript.
    let parent = jsonl_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    let mut w = std::io::BufWriter::new(temporary.as_file_mut());
    for seg in segments {
        if !seg.confidence.is_finite()
            || seg
                .speaker_confidence
                .is_some_and(|value| !value.is_finite())
        {
            return Err(std::io::Error::other("Transcript contains invalid confidence").into());
        }
        let line = serde_json::to_string(seg).map_err(LogError::from)?;
        writeln!(w, "{line}")?;
    }
    w.flush()?;
    drop(w);
    temporary.as_file().sync_all()?;
    temporary
        .persist(jsonl_path)
        .map_err(|error| LogError::Io(error.error))?;
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(id: &str) -> TranscriptSegment {
        TranscriptSegment {
            id: id.into(),
            meeting_id: "m-1".into(),
            speaker_id: None,
            start_ms: 0,
            end_ms: 1_000,
            text: "hello".into(),
            confidence: 1.0,
            partial: false,
            speaker_confidence: None,
        }
    }

    #[test]
    fn append_then_load_roundtrip() {
        let path =
            std::env::temp_dir().join(format!("bruhs-jsonl-rt-{}.jsonl", uuid::Uuid::new_v4()));
        append_segment(&path, &fixture("s-1")).unwrap();
        append_segment(&path, &fixture("s-2")).unwrap();
        let loaded = load_segments(&path).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, "s-1");
        assert_eq!(loaded[1].id, "s-2");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_returns_empty_when_missing() {
        let path = std::env::temp_dir().join(format!(
            "bruhs-jsonl-missing-{}.jsonl",
            uuid::Uuid::new_v4()
        ));
        let loaded = load_segments(&path).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn write_all_replaces_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path();
        append_segment(path, &fixture("a")).unwrap();
        append_segment(path, &fixture("b")).unwrap();
        let mut seg = fixture("a");
        seg.speaker_id = Some("me".into());
        write_all(path, &[seg]).unwrap();
        let loaded = load_segments(path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].speaker_id.as_deref(), Some("me"));
    }

    #[test]
    fn load_skips_malformed_lines() {
        let path =
            std::env::temp_dir().join(format!("bruhs-jsonl-bad-{}.jsonl", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"not json\n{\"id\":\"s-1\",\"meeting_id\":\"m\",\"speaker_id\":null,\"start_ms\":0,\"end_ms\":1,\"text\":\"x\",\"confidence\":1.0,\"partial\":false}\n").unwrap();
        let loaded = load_segments(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "s-1");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn synchronization_does_not_treat_a_damaged_line_as_a_smaller_transcript() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        append_segment(file.path(), &fixture("original")).unwrap();
        file.write_all(b"{incomplete\n").unwrap();
        let original = std::fs::read(file.path()).unwrap();
        assert!(load_snapshot(file.path()).is_err());
        assert_eq!(std::fs::read(file.path()).unwrap(), original);
    }

    #[test]
    fn failed_diarization_write_preserves_the_complete_original() {
        let file = tempfile::NamedTempFile::new().unwrap();
        append_segment(file.path(), &fixture("original")).unwrap();
        let original = std::fs::read(file.path()).unwrap();
        let mut invalid = fixture("bad");
        invalid.confidence = f32::NAN;
        assert!(write_all(file.path(), &[fixture("would-have-replaced"), invalid]).is_err());
        assert_eq!(std::fs::read(file.path()).unwrap(), original);
    }
}
