use serde::Serialize;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};

struct Counter;
static ENABLED: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static REALLOCS: AtomicUsize = AtomicUsize::new(0);
static FREES: AtomicUsize = AtomicUsize::new(0);
static REQUESTED: AtomicUsize = AtomicUsize::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

#[global_allocator]
static ALLOCATOR: Counter = Counter;

fn allocated(bytes: usize) {
    ALLOCS.fetch_add(1, Relaxed);
    REQUESTED.fetch_add(bytes, Relaxed);
    let live = LIVE.fetch_add(bytes, Relaxed) + bytes;
    PEAK.fetch_max(live, Relaxed);
}

// SAFETY: Every allocation operation delegates the exact pointer and layout to
// System. Nonallocating atomic counters do not change ownership or alignment.
unsafe impl GlobalAlloc for Counter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: GlobalAlloc's caller supplies a valid nonzero allocation layout.
        let pointer = unsafe { System.alloc(layout) };
        if ENABLED.load(Relaxed) && !pointer.is_null() {
            allocated(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The caller supplies a valid layout; System preserves zeroing.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if ENABLED.load(Relaxed) && !pointer.is_null() {
            allocated(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if ENABLED.load(Relaxed) {
            FREES.fetch_add(1, Relaxed);
            LIVE.fetch_sub(layout.size(), Relaxed);
        }
        // SAFETY: The caller supplies the live pointer and its original layout.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: The caller supplies a live allocation and valid new size.
        let result = unsafe { System.realloc(pointer, layout, new_size) };
        if ENABLED.load(Relaxed) && !result.is_null() {
            REALLOCS.fetch_add(1, Relaxed);
            REQUESTED.fetch_add(new_size, Relaxed);
            LIVE.fetch_sub(layout.size(), Relaxed);
            let live = LIVE.fetch_add(new_size, Relaxed) + new_size;
            PEAK.fetch_max(live, Relaxed);
        }
        result
    }
}

#[derive(Serialize)]
pub(super) struct Counts {
    allocations: usize,
    reallocations: usize,
    deallocations: usize,
    total_requested_bytes: usize,
    peak_live_requested_bytes: usize,
    live_bytes_at_end: usize,
}

pub(super) fn start() {
    assert!(!ENABLED.load(Relaxed));
    for counter in [&ALLOCS, &REALLOCS, &FREES, &REQUESTED, &LIVE, &PEAK] {
        counter.store(0, Relaxed);
    }
    ENABLED.store(true, Relaxed);
}

pub(super) fn finish() -> Counts {
    ENABLED.store(false, Relaxed);
    let counts = Counts {
        allocations: ALLOCS.load(Relaxed),
        reallocations: REALLOCS.load(Relaxed),
        deallocations: FREES.load(Relaxed),
        total_requested_bytes: REQUESTED.load(Relaxed),
        peak_live_requested_bytes: PEAK.load(Relaxed),
        live_bytes_at_end: LIVE.load(Relaxed),
    };
    assert_eq!(
        counts.live_bytes_at_end, 0,
        "Measured request must free all allocations before counters stop"
    );
    assert_eq!(counts.allocations, counts.deallocations);
    counts
}
