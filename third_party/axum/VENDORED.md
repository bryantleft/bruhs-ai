# Axum SSE byte-writer adapter

Axum 0.8.9 is vendored so task snapshots can batch serializer writes through the existing byte-oriented SSE framing code. The new `EventDataWriter::as_io_write` method borrows that writer as `std::io::Write`. It adds no default buffer or serialization policy.

The cached crates.io archive's SHA-256 matches the original workspace lockfile checksum, `31b698c5f9a010f6573133b09e0de5408834d0c82f8d7475a89fc1867a71cd90`. Its bundled `.cargo_vcs_info.json` records git commit `c59208c86fded335cd85e388030ad59347b0e5ae` (`axum` subdirectory). All 88 archive files were compared with this copy; only the two files below differ. The local note is additional. The MIT license is retained in [LICENSE](LICENSE).

The local changes are limited to:

- `src/response/sse.rs` adds the borrowed byte-writer adapter and reuses it inside the existing `json_data` method. The adapter delegates to the same `write_buf` implementation that `json_data` previously used privately.
- `Cargo.toml` gains an empty standalone workspace declaration. Registry cache-status/checksum files are omitted because the source has a local patch.

The default `json_data` method still writes without buffering. Task-stream callers choose their buffer. The adapter preserves SSE line framing and releases its mutable borrow when dropped. Its own `flush` is a no-op because it writes directly into the event's memory buffer. An outer `BufWriter` must be flushed before `into_event`, or bytes still staged there will be omitted.

Within a function returning `Result<Event, axum::Error>`:

```rust
let mut event = Event::default().into_data_writer();
{
    let mut buffer = BufWriter::with_capacity(8192, event.as_io_write());
    serde_json::to_writer(&mut buffer, &data).map_err(axum::Error::new)?;
    buffer.flush().map_err(axum::Error::new)?;
}
Ok(event.into_event())
```

Serializer failures return an error and drop the partially built event; the task stream yields no partial frame. UTF-8 is not checked here or later. Individual writes may split a scalar, but the completed payload must contain valid UTF-8; otherwise invalid bytes can reach the wire. The task caller uses `serde_json`, which supplies UTF-8. This patch adds no unsafe code.

The [constructor study](../../bench/sse-encoding-probe/README.md) passes complete raw-frame and error-parity checks for named/unnamed events, escaped text, large ASCII/Unicode fields, pretty raw JSON and partial failures. Allocation counters run separately from latency samples. The 8 KiB starting configuration follows a 256-16,384-byte bounded-constructor sweep and is then validated for the byte adapter; this does not establish a universal optimum. It avoids a full JSON temporary, although output-buffer growth can still raise or lower peak requested bytes. The buffer does not cap event size. Actual HTTP results are tracked separately in [the performance audit](../../docs/performance-goal-2026-11-09.md#buffered-task-event-encoding).

When refreshing the dependency:

1. Copy the desired upstream release and retain its license and provenance.
2. Reapply or remove the byte-writer patch after checking the upstream API.
3. Run raw-frame/error parity tests and separate allocation/constructor studies.
4. Run the workspace gate and paired actual HTTP task/combined-stream measurements before retaining a performance change.
