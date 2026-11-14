//! Instrumented startup diagnosis; these observations never establish unprofiled gains.
use super::*;
use std::{mem::MaybeUninit, time::Duration};

struct CpuSnapshot {
    wall: Instant,
    thread: std::thread::ThreadId,
    thread_cpu: Duration,
    process_cpu: Duration,
    minor_faults: u64,
    major_faults: u64,
    voluntary_switches: u64,
    involuntary_switches: u64,
}

fn timeval_duration(value: libc::timeval) -> Duration {
    let seconds = u64::try_from(value.tv_sec).expect("CPU seconds are nonnegative");
    let micros = u32::try_from(value.tv_usec).expect("CPU microseconds are nonnegative");
    assert!(micros < 1_000_000);
    Duration::new(seconds, micros * 1_000)
}

impl CpuSnapshot {
    fn capture() -> Self {
        let mut time = MaybeUninit::<libc::timespec>::uninit();
        // SAFETY: time points to writable timespec storage for the synchronous clock call.
        let status =
            unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, time.as_mut_ptr()) };
        assert_eq!(status, 0, "thread CPU clock must be available");
        // SAFETY: clock_gettime succeeded and initialized both timespec fields.
        let time = unsafe { time.assume_init() };
        // SAFETY: rusage contains integer/timeval fields for which zero is a valid initial value.
        let mut usage = unsafe { std::mem::zeroed::<libc::rusage>() };
        // SAFETY: usage is writable initialized storage that lives through the synchronous call.
        let status = unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
        assert_eq!(status, 0, "process usage must be available");
        Self {
            wall: Instant::now(),
            thread: std::thread::current().id(),
            thread_cpu: Duration::new(
                u64::try_from(time.tv_sec).expect("thread CPU seconds are nonnegative"),
                u32::try_from(time.tv_nsec).expect("thread CPU nanoseconds are nonnegative"),
            ),
            process_cpu: timeval_duration(usage.ru_utime) + timeval_duration(usage.ru_stime),
            minor_faults: u64::try_from(usage.ru_minflt).expect("fault counter is nonnegative"),
            major_faults: u64::try_from(usage.ru_majflt).expect("fault counter is nonnegative"),
            voluntary_switches: u64::try_from(usage.ru_nvcsw)
                .expect("context-switch counter is nonnegative"),
            involuntary_switches: u64::try_from(usage.ru_nivcsw)
                .expect("context-switch counter is nonnegative"),
        }
    }

    fn since(&self, start: &Self) -> Value {
        assert_eq!(
            self.thread, start.thread,
            "thread CPU clocks share a thread"
        );
        let thread = self
            .thread_cpu
            .checked_sub(start.thread_cpu)
            .expect("thread CPU time is monotonic");
        let process = self
            .process_cpu
            .checked_sub(start.process_cpu)
            .expect("process CPU time is monotonic");
        json!({
            "wall_ms":self.wall.duration_since(start.wall).as_secs_f64()*1000.0,
            "thread_cpu_ms":thread.as_secs_f64()*1000.0,
            "process_cpu_ms":process.as_secs_f64()*1000.0,
            "minor_faults":self.minor_faults.checked_sub(start.minor_faults).unwrap(),
            "major_faults":self.major_faults.checked_sub(start.major_faults).unwrap(),
            "voluntary_switches":self.voluntary_switches.checked_sub(start.voluntary_switches).unwrap(),
            "involuntary_switches":self.involuntary_switches.checked_sub(start.involuntary_switches).unwrap(),
        })
    }
}

#[test]
fn native_diagnostic_clocks_advance_on_foreground_work() {
    let before = CpuSnapshot::capture();
    let mut sum = 0_u64;
    for value in 0..100_000_u64 {
        sum = std::hint::black_box(sum.wrapping_add(value));
    }
    std::hint::black_box(sum);
    let after = CpuSnapshot::capture();
    assert!(after.thread_cpu > before.thread_cpu);
    assert!(after.process_cpu >= before.process_cpu);
    let metrics = after.since(&before);
    assert!(metrics["wall_ms"].as_f64().unwrap() > 0.0);
    assert!(metrics["thread_cpu_ms"].as_f64().unwrap() > 0.0);
}

// Keep counter reads out of the observer used for unprofiled performance comparisons.
#[inline(never)]
fn observe_with_counters(
    cx: &mut TestAppContext,
    path: &Path,
    restore: RestoreRecords,
    restore_threads: super::super::ThreadRestorer,
    load: LoadState,
) -> (Observation, Value) {
    let captured = Rc::new(RefCell::new(None));
    let capture = captured.clone();
    let (root, visual) = cx.add_window_view(|window, cx| {
        let started = Instant::now();
        let unix_start_ns = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        )
        .unwrap();
        let usage_start = CpuSnapshot::capture();
        let loaded = load(path);
        let expected_version = loaded.2.clone();
        let usage_read = CpuSnapshot::capture();
        let read_decode_hash_ms = started.elapsed().as_secs_f64() * 1000.0;
        let workspace =
            cx.new(|cx| Workspace::from_loaded_state(loaded, window, cx, restore, restore_threads));
        let root = gpui_component::Root::new(workspace.clone(), window, cx);
        let usage_constructed = CpuSnapshot::capture();
        let constructed = started.elapsed().as_secs_f64() * 1000.0;
        *capture.borrow_mut() = Some((
            workspace,
            started,
            read_decode_hash_ms,
            constructed,
            expected_version,
            unix_start_ns,
            usage_start,
            usage_read,
            usage_constructed,
        ));
        root
    });
    // add_window_view runs the test executor until parked, including the initial headless frame.
    let (
        workspace,
        started,
        read_decode_hash_ms,
        constructed,
        expected_version,
        unix_start_ns,
        usage_start,
        usage_read,
        usage_constructed,
    ) = captured.borrow_mut().take().unwrap();
    let usage_settled = CpuSnapshot::capture();
    let total_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert!(visual.debug_bounds("desktop-composer").is_some());
    let (snapshot, recovered) = workspace.read_with(visual, |workspace, cx| {
        assert_eq!(
            *workspace.persistence.write_lock.lock().unwrap(),
            expected_version
        );
        assert_eq!(
            workspace.persist_requests, 0,
            "startup must not save or run a workflow"
        );
        assert!(workspace
            .threads
            .iter()
            .all(|thread| !thread.chat.read(cx).is_streaming));
        assert!(workspace.threads.iter().all(|thread| {
            let chat = thread.chat.read(cx);
            chat.prompt_queue.is_empty() || chat.queue_held
        }));
        (
            serde_json::to_value(workspace.snapshot(cx)).unwrap(),
            matches!(workspace.save_status, SaveStatus::RecoveryRequired(_)),
        )
    });
    super::super::super::cleanup::close_fixture(root, workspace, visual);
    (
        Observation {
            total_ms,
            read_decode_hash_ms,
            construct_ms: constructed - read_decode_hash_ms,
            settle_ms: total_ms - constructed,
            snapshot,
            recovered,
        },
        json!({"unix_start_ns":unix_start_ns,"total":usage_settled.since(&usage_start),"load":usage_read.since(&usage_start),"construct":usage_constructed.since(&usage_read),"settle":usage_settled.since(&usage_constructed)}),
    )
}

#[gpui::test]
#[ignore = "manual instrumented wall/thread/process CPU and resource diagnosis, excluded from latency gains"]
fn workspace_settling_cpu_diagnostic(cx: &mut TestAppContext) {
    initialize(cx);
    let mut capture_samples_ms = Vec::with_capacity(1_000);
    for _ in 0..1_000 {
        let started = Instant::now();
        std::hint::black_box(CpuSnapshot::capture());
        capture_samples_ms.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    println!(
        "CPU_COUNTER_CALIBRATION {}",
        json!({"capture_samples_ms":capture_samples_ms,"thread_clock_id":libc::CLOCK_THREAD_CPUTIME_ID,"process_usage":"RUSAGE_SELF","scope":"Counter-read cost only, not application latency; no overhead is subtracted from observations."})
    );
    let (variant, restore): (&str, ThreadRestorer) =
        match std::env::var("BRUHS_PERF_PROJECT_PATH").as_deref() {
            Ok("original") => (
                "original",
                super::super::super::project_path_reference::default_threads,
            ),
            Ok("known") | Err(_) => ("known", threads::current_threads),
            other => panic!("invalid project path construction {other:?}"),
        };
    let selected = std::env::var("BRUHS_PERF_PROJECT_PATH_CASE").ok();
    let root = tempfile::tempdir().unwrap();
    let mut measured = 0;
    for label in CASES {
        if selected
            .as_deref()
            .is_some_and(|selected| selected != label)
        {
            continue;
        }
        let bytes = fixture_bytes(label);
        for recovery in [false, true] {
            let path = root.path().join(format!("{label}-{recovery}.json"));
            if recovery {
                fs::write(&path, b"{ invalid primary").unwrap();
                fs::write(desktop_state_backup_path(&path), &bytes).unwrap();
            } else {
                fs::write(&path, &bytes).unwrap();
            }
            let expected = observe_with_loader(
                cx,
                &path,
                current,
                super::super::super::project_path_reference::default_threads,
                load_desktop_state_with_backup,
            );
            let mut samples = Vec::new();
            for iteration in 0..35 {
                let (actual, metrics) = observe_with_counters(
                    cx,
                    &path,
                    current,
                    restore,
                    load_desktop_state_with_backup,
                );
                assert_eq!(
                    actual.snapshot, expected.snapshot,
                    "{label}/{recovery}/{iteration}"
                );
                assert_eq!(actual.recovered, recovery);
                samples.push(json!({"iteration":iteration,"warmup":iteration<5,"metrics":metrics,"observed_total_ms":actual.total_ms,"observed_load_ms":actual.read_decode_hash_ms,"observed_construct_ms":actual.construct_ms,"observed_settle_ms":actual.settle_ms}));
            }
            assert_eq!(
                fs::read(if recovery {
                    desktop_state_backup_path(&path)
                } else {
                    path.clone()
                })
                .unwrap(),
                bytes
            );
            if recovery {
                assert_eq!(fs::read(&path).unwrap(), b"{ invalid primary");
            }
            println!(
                "SETTLE_CPU_DIAGNOSTIC {}",
                json!({"label":label,"recovery":recovery,"variant":variant,"input_bytes":bytes.len(),"input_sha256":format!("{:x}",Sha256::digest(&bytes)),"warmups":5,"samples":samples})
            );
            measured += 1;
        }
    }
    assert!(measured > 0, "selected diagnostic fixture must exist");
}
