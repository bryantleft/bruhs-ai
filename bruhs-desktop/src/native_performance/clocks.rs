use serde_json::{json, Value};
use std::{mem::MaybeUninit, time::Duration};

pub(super) struct CpuClock {
    thread: std::thread::ThreadId,
    thread_cpu: Duration,
    process_cpu: Duration,
}

fn duration(value: libc::timeval) -> Duration {
    Duration::new(
        u64::try_from(value.tv_sec).expect("nonnegative CPU seconds"),
        u32::try_from(value.tv_usec).expect("nonnegative CPU microseconds") * 1_000,
    )
}

impl CpuClock {
    pub(super) fn read() -> Self {
        let mut thread = MaybeUninit::<libc::timespec>::uninit();
        // SAFETY: the synchronous call receives writable timespec storage.
        let status =
            unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, thread.as_mut_ptr()) };
        assert_eq!(status, 0);
        // SAFETY: successful clock_gettime initialized the timespec fields.
        let thread = unsafe { thread.assume_init() };
        let mut usage = MaybeUninit::<libc::rusage>::uninit();
        // SAFETY: the synchronous call receives writable rusage storage.
        let status = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
        assert_eq!(status, 0);
        // SAFETY: successful getrusage initialized the rusage fields we read.
        let usage = unsafe { usage.assume_init() };
        Self {
            thread: std::thread::current().id(),
            thread_cpu: Duration::new(
                u64::try_from(thread.tv_sec).expect("CPU seconds"),
                u32::try_from(thread.tv_nsec).expect("CPU nanoseconds"),
            ),
            process_cpu: duration(usage.ru_utime) + duration(usage.ru_stime),
        }
    }

    pub(super) fn since(&self, before: &Self) -> Value {
        assert_eq!(self.thread, before.thread);
        json!({
            "thread_cpu_ms":self.thread_cpu.checked_sub(before.thread_cpu).expect("monotonic CPU").as_secs_f64()*1_000.0,
            "process_cpu_ms":self.process_cpu.checked_sub(before.process_cpu).expect("monotonic process CPU").as_secs_f64()*1_000.0,
        })
    }
}
