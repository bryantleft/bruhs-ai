use super::*;
use std::{io::Cursor, time::Duration};
use uuid::Uuid;

fn at(ms: u64) -> Duration {
    Duration::from_millis(ms)
}
fn binding() -> MuteBinding {
    MuteBinding {
        capture_generation: Uuid::new_v4(),
        observer_generation: Uuid::new_v4(),
    }
}
fn frame(start: u64, end: u64) -> Option<SampleWindow> {
    Some(SampleWindow {
        start: at(start),
        end: at(end),
        received: at(end),
    })
}
fn guarded() -> (MicrophoneGate, MuteBinding) {
    let mut gate = MicrophoneGate::ordinary();
    let binding = gate.enable(Uuid::new_v4(), at(0));
    (gate, binding)
}
fn spec(channels: u16) -> hound::WavSpec {
    hound::WavSpec {
        channels,
        sample_rate: 8,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    }
}
fn pcm(cursor: &mut Cursor<Vec<u8>>) -> Vec<i16> {
    cursor.set_position(0);
    hound::WavReader::new(cursor)
        .unwrap()
        .samples::<i16>()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn guarded_capture_requires_fresh_unmuted_and_timestamped_audio() {
    let (mut gate, b) = guarded();
    assert_eq!(gate.decision(frame(10, 20)), MicDecision::Unknown);
    gate.observe(b, 1, at(10), at(10), MuteSignal::Unmuted);
    assert_eq!(gate.decision(None), MicDecision::TimingUnavailable);
    assert_eq!(gate.decision(frame(9, 20)), MicDecision::TimingUnavailable);
    assert_eq!(gate.decision(frame(10, 20)), MicDecision::Capture);
    assert_eq!(gate.decision(frame(10, 2010)), MicDecision::Stale);
    gate.observe(b, 2, at(2020), at(2020), MuteSignal::Unmuted);
    assert_eq!(
        gate.decision(frame(2019, 2021)),
        MicDecision::TimingUnavailable
    );
    assert_eq!(gate.decision(frame(2020, 2021)), MicDecision::Capture);
}

#[test]
fn foreign_and_old_observations_cannot_reopen_a_muted_session() {
    let (mut gate, b) = guarded();
    gate.observe(b, 10, at(10), at(10), MuteSignal::Muted);
    for foreign in [
        binding(),
        MuteBinding {
            observer_generation: Uuid::new_v4(),
            ..b
        },
        MuteBinding {
            capture_generation: Uuid::new_v4(),
            ..b
        },
    ] {
        assert!(!gate.observe(foreign, 11, at(11), at(11), MuteSignal::Unmuted));
    }
    for old in [0, 9, 10] {
        assert!(!gate.observe(b, old, at(11), at(11), MuteSignal::Unmuted));
    }
    assert_eq!(gate.decision(frame(11, 12)), MicDecision::Muted);
}

#[test]
fn invalid_newer_observation_closes_gate_without_preventing_later_recovery() {
    for (observed, received) in [(5, 11), (100_000, 11), (11, 2011)] {
        let (mut gate, b) = guarded();
        gate.observe(b, 1, at(10), at(10), MuteSignal::Unmuted);
        gate.observe(b, 2, at(observed), at(received), MuteSignal::Unmuted);
        assert_eq!(gate.decision(frame(11, 12)), MicDecision::Unknown);
        gate.observe(b, 3, at(3000), at(3000), MuteSignal::Unmuted);
        assert_eq!(gate.decision(frame(3000, 3001)), MicDecision::Capture);
    }
}

#[test]
fn manual_pause_survives_enable_unmute_and_opt_out() {
    let (mut gate, b) = guarded();
    gate.pause(true, at(10));
    let b = gate.enable(b.capture_generation, at(11));
    gate.observe(b, 1, at(12), at(12), MuteSignal::Unmuted);
    gate.disable(at(13));
    assert_eq!(gate.decision(frame(13, 14)), MicDecision::ManualPause);
    gate.pause(false, at(15));
    assert_eq!(gate.decision(frame(14, 16)), MicDecision::TimingUnavailable);
    assert_eq!(gate.decision(frame(15, 16)), MicDecision::Capture);
}

#[test]
fn revoke_requires_explicit_enable_and_new_session_cannot_reuse_signal() {
    let (mut gate, b) = guarded();
    gate.observe(b, 1, at(10), at(10), MuteSignal::Unmuted);
    gate.revoke();
    assert!(!gate.observe(b, 2, at(11), at(11), MuteSignal::Unmuted));
    assert_eq!(gate.decision(frame(11, 12)), MicDecision::Revoked);
    gate.enable(Uuid::new_v4(), at(13));
    assert!(!gate.observe(b, 3, at(14), at(14), MuteSignal::Unmuted));
    assert_eq!(gate.decision(frame(14, 15)), MicDecision::Unknown);
}

#[test]
fn reenable_in_same_capture_rejects_previous_observer_even_with_newer_sequence() {
    let (mut gate, old) = guarded();
    gate.observe(old, 1, at(10), at(10), MuteSignal::Unmuted);
    gate.disable(at(20));
    let new = gate.enable(old.capture_generation, at(30));
    assert_ne!(old.observer_generation, new.observer_generation);
    assert!(!gate.observe(old, 100, at(31), at(31), MuteSignal::Unmuted));
    assert_eq!(gate.decision(frame(31, 32)), MicDecision::Unknown);
    assert!(gate.observe(new, 1, at(32), at(32), MuteSignal::Unmuted));
    assert_eq!(gate.decision(frame(32, 33)), MicDecision::Capture);
}

#[test]
fn queued_frames_never_return_after_unknown_muted_or_explicit_opt_out() {
    for signal in [MuteSignal::Muted, MuteSignal::Unknown] {
        let (mut gate, b) = guarded();
        gate.observe(b, 1, at(10), at(10), signal);
        gate.observe(b, 2, at(20), at(25), MuteSignal::Unmuted);
        assert_eq!(gate.decision(frame(24, 26)), MicDecision::TimingUnavailable);
        assert_eq!(gate.decision(frame(25, 26)), MicDecision::Capture);
        gate.disable(at(30));
        assert_eq!(gate.decision(None), MicDecision::TimingUnavailable);
        assert_eq!(gate.decision(frame(29, 31)), MicDecision::TimingUnavailable);
        assert_eq!(gate.decision(frame(30, 31)), MicDecision::Capture);
    }
}

#[test]
fn inverted_or_future_frame_bounds_are_silenced() {
    let (mut gate, b) = guarded();
    gate.observe(b, 1, at(10), at(10), MuteSignal::Unmuted);
    for frame in [
        SampleWindow {
            start: at(12),
            end: at(11),
            received: at(13),
        },
        SampleWindow {
            start: at(10),
            end: at(15),
            received: at(14),
        },
    ] {
        assert_eq!(gate.decision(Some(frame)), MicDecision::TimingUnavailable);
    }
}

#[test]
fn both_pcm_formats_zero_before_wav_and_meter_without_shortening_timeline() {
    for floats in [false, true] {
        let mut bytes = Cursor::new(Vec::new());
        let (gate, b) = guarded();
        let mut writer =
            MicrophoneWriter::new(hound::WavWriter::new(&mut bytes, spec(1)).unwrap(), gate)
                .unwrap();
        let write = |writer: &mut MicrophoneWriter<_>, time| {
            if floats {
                writer.write_f32(&[1.0, -1.0], frame(time, time + 1))
            } else {
                writer.write_i16(&[i16::MAX, -i16::MAX], frame(time, time + 1))
            }
        };
        assert_eq!(write(&mut writer, 0), Some(0.0)); // Unknown.
        writer
            .gate_mut()
            .observe(b, 1, at(10), at(10), MuteSignal::Unmuted);
        assert!(write(&mut writer, 10).unwrap() > 0.99);
        writer
            .gate_mut()
            .observe(b, 2, at(20), at(20), MuteSignal::Muted);
        assert_eq!(write(&mut writer, 20), Some(0.0));
        writer
            .gate_mut()
            .observe(b, 3, at(30), at(30), MuteSignal::Unmuted);
        assert_eq!(write(&mut writer, 2030), Some(0.0)); // Lease expired.
        writer.finalize().unwrap();
        assert_eq!(pcm(&mut bytes), vec![0, 0, i16::MAX, -i16::MAX, 0, 0, 0, 0]);
    }
}

#[test]
fn rms_forgets_old_energy_even_when_pause_and_resume_happen_between_callbacks() {
    let mut bytes = Cursor::new(Vec::new());
    let mut writer = MicrophoneWriter::new(
        hound::WavWriter::new(&mut bytes, spec(1)).unwrap(),
        MicrophoneGate::ordinary(),
    )
    .unwrap();
    assert_eq!(writer.write_i16(&[i16::MAX], None), None);
    writer.gate_mut().pause(true, at(10));
    writer.gate_mut().pause(false, at(20));
    assert_eq!(writer.write_i16(&[0], frame(20, 21)), None);
    assert_eq!(writer.write_i16(&[0], frame(21, 22)), Some(0.0));
    writer.finalize().unwrap();
}

#[test]
fn fresh_heartbeats_do_not_reset_meter_and_stereo_uses_sample_frames() {
    let mut bytes = Cursor::new(Vec::new());
    let (gate, b) = guarded();
    let mut writer =
        MicrophoneWriter::new(hound::WavWriter::new(&mut bytes, spec(2)).unwrap(), gate).unwrap();
    writer
        .gate_mut()
        .observe(b, 1, at(0), at(0), MuteSignal::Unmuted);
    assert_eq!(writer.write_i16(&[16384, 16384], frame(0, 1)), None);
    writer
        .gate_mut()
        .observe(b, 2, at(2), at(2), MuteSignal::Unmuted);
    assert_eq!(writer.write_i16(&[16384, 16384], frame(2, 3)), Some(0.5));
    writer.finalize().unwrap();
}

#[test]
fn ordinary_capture_preserves_pcm_and_sanitizes_nonfinite_float_input() {
    let mut bytes = Cursor::new(Vec::new());
    let mut writer = MicrophoneWriter::new(
        hound::WavWriter::new(&mut bytes, spec(1)).unwrap(),
        MicrophoneGate::ordinary(),
    )
    .unwrap();
    writer.write_i16(&[i16::MIN, 123, i16::MAX], None);
    let level = writer
        .write_f32(&[f32::NAN, f32::INFINITY, -2.0, 2.0], None)
        .unwrap();
    assert!(level.is_finite() && (0.0..=1.0).contains(&level));
    writer.finalize().unwrap();
    assert_eq!(
        pcm(&mut bytes),
        vec![i16::MIN, 123, i16::MAX, 0, 0, -i16::MAX, i16::MAX]
    );
}

#[test]
fn microphone_gate_does_not_touch_a_separate_remote_audio_writer() {
    let mut mic_bytes = Cursor::new(Vec::new());
    let mut remote_bytes = Cursor::new(Vec::new());
    let (gate, _) = guarded();
    let mut mic = MicrophoneWriter::new(
        hound::WavWriter::new(&mut mic_bytes, spec(1)).unwrap(),
        gate,
    )
    .unwrap();
    let mut remote = hound::WavWriter::new(&mut remote_bytes, spec(1)).unwrap();
    mic.write_i16(&[1000, 2000, 3000, 4000], None);
    for sample in [1000i16, 2000, 3000, 4000] {
        remote.write_sample(sample).unwrap();
    }
    mic.finalize().unwrap();
    remote.finalize().unwrap();
    assert_eq!(pcm(&mut mic_bytes), vec![0; 4]);
    assert_eq!(pcm(&mut remote_bytes), vec![1000, 2000, 3000, 4000]);
}

#[test]
fn failed_pcm_write_stops_future_writes_and_survives_successful_finalization() {
    use std::{cell::Cell, io, rc::Rc};

    struct FailingSink {
        bytes: Cursor<Vec<u8>>,
        fail: Rc<Cell<bool>>,
        writes: Rc<Cell<usize>>,
    }
    impl Write for FailingSink {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.writes.set(self.writes.get() + 1);
            if self.fail.get() {
                return Err(io::Error::other("synthetic disk failure"));
            }
            self.bytes.write(data)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl Seek for FailingSink {
        fn seek(&mut self, position: io::SeekFrom) -> io::Result<u64> {
            self.bytes.seek(position)
        }
    }
    let fail = Rc::new(Cell::new(false));
    let writes = Rc::new(Cell::new(0));
    let sink = FailingSink {
        bytes: Cursor::new(Vec::new()),
        fail: Rc::clone(&fail),
        writes: Rc::clone(&writes),
    };
    let mut writer = MicrophoneWriter::new(
        hound::WavWriter::new(sink, spec(1)).unwrap(),
        MicrophoneGate::ordinary(),
    )
    .unwrap();
    fail.set(true);
    assert_eq!(writer.write_i16(&[123, 456], None), None);
    assert!(writer
        .take_failure_notice()
        .unwrap()
        .to_string()
        .contains("synthetic disk failure"));
    assert!(writer.take_failure_notice().is_none());
    writer.fail_stream("later stream failure".into());
    let after_failure = writes.get();
    fail.set(false);
    assert_eq!(writer.write_f32(&[1.0, 1.0], None), None);
    assert_eq!(writes.get(), after_failure);
    assert!(writer
        .finalize()
        .unwrap_err()
        .to_string()
        .contains("synthetic disk failure"));
    assert!(
        writes.get() > after_failure,
        "WAV header must still finalize"
    );
}

#[test]
fn stream_failure_prevents_writes_and_survives_notice_and_gate_changes() {
    let mut bytes = Cursor::new(Vec::new());
    let mut writer = MicrophoneWriter::new(
        hound::WavWriter::new(&mut bytes, spec(1)).unwrap(),
        MicrophoneGate::ordinary(),
    )
    .unwrap();
    writer.write_i16(&[123], None);
    assert!(writer.take_failure_notice().is_none());
    writer.fail_stream("device disconnected".into());
    writer.fail_stream("later error".into());
    assert!(writer
        .take_failure_notice()
        .unwrap()
        .to_string()
        .contains("device disconnected"));
    assert!(writer.take_failure_notice().is_none());
    writer.gate_mut().disable(Duration::ZERO);
    assert_eq!(writer.write_f32(&[1.0; 16], None), None);
    assert!(writer
        .finalize()
        .unwrap_err()
        .to_string()
        .contains("device disconnected"));
    assert_eq!(pcm(&mut bytes), vec![123]);
}

#[test]
fn writer_rejects_non_pcm16_format() {
    let wav = hound::WavWriter::new(
        Cursor::new(Vec::new()),
        hound::WavSpec {
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
            ..spec(1)
        },
    )
    .unwrap();
    assert!(matches!(
        MicrophoneWriter::new(wav, MicrophoneGate::ordinary()),
        Err(MicrophoneWriteError::InvalidFormat)
    ));
}
