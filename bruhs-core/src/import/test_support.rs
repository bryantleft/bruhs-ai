//! Helpers for import tests in this crate and in the server's tests
//! (`test-support` feature): export archives built in code, and the
//! synthetic fixtures under `src/import/fixtures/`.

use std::io::{Cursor, Write as _};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// A ZIP holding `entries` in order, deflated or stored.
///
/// # Panics
///
/// When the writer refuses an entry (a repeated name).
pub fn zip_of(entries: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
    let method = if deflate {
        CompressionMethod::Deflated
    } else {
        CompressionMethod::Stored
    };
    let options = SimpleFileOptions::default()
        .compression_method(method)
        .large_file(false);
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        writer
            .start_file(*name, options)
            .expect("a test entry starts");
        writer.write_all(bytes).expect("a test entry is written");
    }
    writer.finish().expect("a test ZIP finishes").into_inner()
}

/// Every fixture under `src/import/fixtures/`, by file name. All are
/// hand-written and synthetic (`fixtures_are_synthetic`).
const FIXTURES: &[(&str, &[u8])] = &[
    (
        "chatgpt-conversations.json",
        include_bytes!("fixtures/chatgpt-conversations.json"),
    ),
    (
        "chatgpt-export_manifest.json",
        include_bytes!("fixtures/chatgpt-export_manifest.json"),
    ),
    (
        "claude-conversations.json",
        include_bytes!("fixtures/claude-conversations.json"),
    ),
    (
        "claude-memories.json",
        include_bytes!("fixtures/claude-memories.json"),
    ),
    (
        "claude-memories-account.json",
        include_bytes!("fixtures/claude-memories-account.json"),
    ),
    (
        "claude-manifest.json",
        include_bytes!("fixtures/claude-manifest.json"),
    ),
    (
        "grok-prod-grok-backend.json",
        include_bytes!("fixtures/grok-prod-grok-backend.json"),
    ),
];

/// The fixture file `name`, from `src/import/fixtures/`.
///
/// # Panics
///
/// When no fixture has that name.
pub fn fixture(name: &str) -> &'static [u8] {
    match FIXTURES.iter().find(|(fixture, _)| *fixture == name) {
        Some((_, bytes)) => bytes,
        None => panic!("no import fixture named {name}"),
    }
}

/// How long one timed run may take before it counts as a hang.
const HANG: std::time::Duration = std::time::Duration::from_secs(60);
/// Runs timed at each size; the fastest counts, so a burst of load on the
/// machine doesn't.
const REPEATS: usize = 3;
/// The smaller size is doubled until its fastest run takes at least this
/// long, so it times well above the clock's and the caches' noise on any
/// machine and in any profile.
const CALIBRATED: std::time::Duration = std::time::Duration::from_millis(20);
/// Doublings of the smaller size at most (256 times the caller's `n`).
const MAX_DOUBLINGS: u32 = 8;
/// Four times the input may take at most this many times as long. Linear
/// work measures 2 to 4.2 here; quadratic work measures 11.7 to 16, not a
/// clean 16, because its linear part dilutes it. So the line sits at 8,
/// well under 16, with room on both sides.
const MAX_RATIO: f64 = 8.0;

/// Asserts `run` grows about linearly with its input, whatever the
/// machine's load. Starting from `n`, the size is doubled until the fastest
/// run on `build(n)` takes at least [`CALIBRATED`]; then runs on `build(n)`
/// and `build(4 * n)` alternate, [`REPEATS`] each, and the fastest at the
/// larger size must take under [`MAX_RATIO`] times the fastest at the
/// smaller. Only `run` is timed, on its own thread, and each run must
/// finish within 60 s, so a hang fails too.
///
/// # Panics
///
/// When the growth is superlinear, a run hangs, or `run` panics.
pub fn assert_scales_linearly<T: Send + Sync + 'static>(
    what: &str,
    n: usize,
    build: impl Fn(usize) -> T,
    run: impl Fn(&T) + Send + Sync + 'static,
) {
    use std::sync::{mpsc, Arc};
    use std::time::Duration;
    let run = Arc::new(run);
    let once = |input: &Arc<T>, size: usize| -> Duration {
        let (done, timed) = mpsc::channel();
        let (run, input) = (Arc::clone(&run), Arc::clone(input));
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            run(&input);
            let _ = done.send(started.elapsed());
        });
        match timed.recv_timeout(HANG) {
            Ok(took) => took,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                panic!("{what} at size {size} didn't finish within {HANG:?}")
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                panic!("{what} at size {size} panicked")
            }
        }
    };
    let fastest = |input: &Arc<T>, size: usize| {
        (0..REPEATS)
            .map(|_| once(input, size))
            .min()
            .unwrap_or(Duration::MAX)
    };
    let mut n = n.max(1);
    let mut small_input = Arc::new(build(n));
    for _ in 0..MAX_DOUBLINGS {
        if fastest(&small_input, n) >= CALIBRATED {
            break;
        }
        n = n.saturating_mul(2);
        small_input = Arc::new(build(n));
    }
    let large_n = n.saturating_mul(4);
    let large_input = Arc::new(build(large_n));
    let (mut small, mut large) = (Duration::MAX, Duration::MAX);
    // Alternated, so a burst of load falls on both sizes.
    for _ in 0..REPEATS {
        small = small.min(once(&small_input, n));
        large = large.min(once(&large_input, large_n));
    }
    // Only a run that never reached the calibrated time is read as it.
    let ratio = large.as_secs_f64() / small.max(CALIBRATED).as_secs_f64();
    assert!(
        ratio < MAX_RATIO,
        "{what} isn't linear: {small:?} at {n}, {large:?} at {large_n} (ratio {ratio:.1})"
    );
}

#[cfg(test)]
mod tests {
    use super::assert_scales_linearly;

    fn work(n: usize) -> u64 {
        (0..n).fold(0_u64, |sum, i| {
            std::hint::black_box(sum.wrapping_add(u64::try_from(i).unwrap_or(0)))
        })
    }

    #[test]
    fn linear_work_passes() {
        assert_scales_linearly(
            "linear work",
            1_000,
            |n| n,
            |n| {
                std::hint::black_box(work(*n));
            },
        );
    }

    #[test]
    #[should_panic(expected = "isn't linear")]
    fn quadratic_work_fails() {
        assert_scales_linearly(
            "quadratic work",
            100,
            |n| n,
            |n| {
                for _ in 0..*n {
                    std::hint::black_box(work(*n));
                }
            },
        );
    }
}
