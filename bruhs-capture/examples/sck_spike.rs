#![expect(
    clippy::useless_transmute,
    reason = "cidre's `define_obj_type!` macro expands to a transmute internally"
)]
#![expect(
    clippy::unwrap_used,
    reason = "exploratory spike, not shipped code: a bare unwrap is the clearest failure"
)]

//! Spike: prove cidre + ScreenCaptureKit can capture system audio to a WAV.
//!
//! Run from `bruhs-capture/`: `cargo run --example sck_spike`
//!
//! Output: `/tmp/bruhs-sck-spike.wav` (mono, 48kHz, 16-bit PCM, 10 seconds).
//!
//! The first run triggers the macOS "Screen Recording" permission prompt;
//! grant it and re-run. Play any audio in another tab/app while it runs.

use bruhs_models::num::{ToFloatLossy, ToIntSaturating};

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("This spike is macOS-only.");
    std::process::exit(1);
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use cidre::{cm, define_obj_type, dispatch, ns, objc, sc};
    use sc::stream::{Output, OutputImpl};

    // ---- Audio writer shared with the stream-output delegate. -----------
    //
    // We capture stereo float32 from SCK, downmix to mono i16, and write to
    // a hound WavWriter at 48kHz. Stop time finalizes the writer.
    struct Writer {
        wav: Option<hound::WavWriter<std::io::BufWriter<std::fs::File>>>,
        frames_written: u64,
        sample_rate: u32,
        channels: u32,
        first_log: bool,
    }

    impl Writer {
        fn finalize(&mut self) -> Result<(), hound::Error> {
            if let Some(w) = self.wav.take() {
                w.finalize()?;
            }
            Ok(())
        }
    }

    // Delegate state. Cidre's `define_obj_type!` macro embeds this inside an
    // Objective-C subclass that conforms to `SCStreamOutput`.
    #[repr(C)]
    struct SpikeOutputInner {
        writer: Arc<Mutex<Writer>>,
    }

    impl SpikeOutputInner {
        fn handle_audio(&mut self, sample_buf: &mut cm::SampleBuf) {
            // SCK emits non-interleaved float32 by default. We read each
            // channel buffer, downmix to mono float, scale to i16, and append
            // to the WAV.
            let Some(format_desc) = sample_buf.format_desc() else {
                return;
            };
            let Some(asbd) = format_desc.stream_basic_desc() else {
                return;
            };
            let channels = asbd.channels_per_frame as usize;
            let sample_rate = asbd.sample_rate.to_u32_saturating();
            let num_frames = usize::try_from(sample_buf.num_samples()).unwrap_or(0);
            if num_frames == 0 || channels == 0 {
                return;
            }

            // Update writer metadata on first frame so the WAV header logging
            // reflects actual SCK output (we still write at 48kHz/mono).
            {
                let mut w = self.writer.lock().unwrap();
                if w.first_log {
                    w.sample_rate = sample_rate;
                    w.channels = u32::try_from(channels).unwrap_or(u32::MAX);
                }
            }

            // Pull the audio data out of the CMSampleBuffer. The returned
            // list owns a retained block buffer — pointers in `buffers` stay
            // valid until `list` is dropped.
            //
            // We expect 2 buffers for stereo non-interleaved (SCK default).
            let list = match sample_buf.audio_buf_list::<2>() {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("audio_buf_list err: {:?}", e);
                    return;
                }
            };
            let buffers = list.list().as_slice();
            convert_and_write(buffers, num_frames, channels, &self.writer);
            drop(list);
        }
    }

    // Helper: read the AudioBuffer pointers, downmix to mono float, write i16.
    fn convert_and_write(
        buffers: &[cidre::cat::audio::Buf],
        num_frames: usize,
        channels: usize,
        writer_mu: &Arc<Mutex<Writer>>,
    ) -> usize {
        // SCK gives non-interleaved float32: one Buf per channel. Each Buf
        // contains `num_frames` f32 samples for that channel.
        if buffers.len() != channels {
            // Some sample buffers may have an interleaved layout in 1-channel
            // configs. Fall back: treat as interleaved.
            return write_interleaved(buffers, num_frames, channels, writer_mu);
        }

        let mut mono = vec![0.0f32; num_frames];
        for ch_buf in buffers {
            if ch_buf.data.is_null() {
                continue;
            }
            let byte_len = ch_buf.data_bytes_size as usize;
            let frame_count = byte_len / std::mem::size_of::<f32>();
            let n = frame_count.min(num_frames);
            // SAFETY: ch_buf.data holds `byte_len >= n * 4` bytes of one
            // channel's f32 samples, owned by the CMSampleBuffer for the
            // duration of this delegate callback. (Spike only: the daemon checks
            // the format and alignment first; see capture/macos.rs.)
            let slice = unsafe { std::slice::from_raw_parts(ch_buf.data as *const f32, n) };
            for (i, &s) in slice.iter().enumerate() {
                mono[i] += s;
            }
        }
        let inv_ch = 1.0_f32 / channels.to_f32_lossy();
        for s in mono.iter_mut() {
            *s *= inv_ch;
        }

        let mut writer = writer_mu.lock().unwrap();
        if writer.first_log {
            eprintln!(
                "first audio buf: {} frames, {} ch, {} Hz",
                num_frames, channels, writer.sample_rate
            );
            writer.first_log = false;
        }
        if let Some(w) = writer.wav.as_mut() {
            for &s in &mono {
                let clamped = (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)).to_i16_saturating();
                if let Err(e) = w.write_sample(clamped) {
                    eprintln!("write_sample err: {e}");
                    return 0;
                }
            }
        }
        writer.frames_written += num_frames as u64;
        num_frames
    }

    fn write_interleaved(
        buffers: &[cidre::cat::audio::Buf],
        num_frames: usize,
        channels: usize,
        writer_mu: &Arc<Mutex<Writer>>,
    ) -> usize {
        // Interleaved path: one Buf with `channels * num_frames` floats.
        let buf = match buffers.first() {
            Some(b) => b,
            None => return 0,
        };
        if buf.data.is_null() {
            return 0;
        }
        let byte_len = buf.data_bytes_size as usize;
        let total = byte_len / std::mem::size_of::<f32>();
        let frames = total / channels;
        let n = frames.min(num_frames);
        // SAFETY: buf.data holds `byte_len >= n * channels * 4` bytes of
        // interleaved f32 samples owned by the CMSampleBuffer, which outlives
        // this delegate callback. (Spike only: the daemon checks the stream
        // format and alignment before reading; see capture/macos.rs.)
        let slice = unsafe { std::slice::from_raw_parts(buf.data as *const f32, n * channels) };
        let mut writer = writer_mu.lock().unwrap();
        if writer.first_log {
            eprintln!(
                "first audio buf (interleaved): {} frames, {} ch, {} Hz",
                n, channels, writer.sample_rate
            );
            writer.first_log = false;
        }
        if let Some(w) = writer.wav.as_mut() {
            let inv_ch = 1.0_f32 / channels.to_f32_lossy();
            for frame in slice.chunks_exact(channels) {
                let sum: f32 = frame.iter().sum();
                let mono = (sum * inv_ch).clamp(-1.0, 1.0);
                let clamped = (mono * f32::from(i16::MAX)).to_i16_saturating();
                if let Err(e) = w.write_sample(clamped) {
                    eprintln!("write_sample err: {e}");
                    return 0;
                }
            }
        }
        writer.frames_written += n as u64;
        n
    }

    define_obj_type!(SpikeOutput + OutputImpl, SpikeOutputInner, SPIKE_OUTPUT);

    impl Output for SpikeOutput {}

    #[objc::add_methods]
    impl OutputImpl for SpikeOutput {
        extern "C" fn impl_stream_did_output_sample_buf(
            &mut self,
            _cmd: Option<&objc::Sel>,
            _stream: &sc::Stream,
            sample_buf: &mut cm::SampleBuf,
            kind: sc::OutputType,
        ) {
            if matches!(kind, sc::OutputType::Audio) {
                self.inner_mut().handle_audio(sample_buf);
            }
        }
    }

    // ---- Main: run the stream for 10 seconds. ---------------------------
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    rt.block_on(async move {
        // Open the WAV writer up front; we'll write to it from the delegate.
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let path = "/tmp/bruhs-sck-spike.wav";
        let _ = std::fs::remove_file(path);
        let wav = hound::WavWriter::create(path, spec)?;

        let writer = Arc::new(Mutex::new(Writer {
            wav: Some(wav),
            frames_written: 0,
            sample_rate: 48_000,
            channels: 1,
            first_log: true,
        }));

        let content = sc::ShareableContent::current()
            .await
            .map_err(|e| format!("ShareableContent::current failed: {:?}", e))?;
        let displays = content.displays();
        let display = displays
            .get(0)
            .map_err(|e| format!("no display available: {:?}", e))?;

        let mut cfg = sc::StreamCfg::new();
        // Audio: enable, exclude our own process so we don't loopback ourselves.
        cfg.set_captures_audio(true);
        cfg.set_excludes_current_process_audio(true);
        cfg.set_sample_rate(48_000);
        cfg.set_channel_count(2);
        // Video: required by SCK even if we only want audio. Keep tiny + slow.
        cfg.set_width(2);
        cfg.set_height(2);
        cfg.set_minimum_frame_interval(cm::Time::new(1, 1));

        let windows = ns::Array::new();
        let filter = sc::ContentFilter::with_display_excluding_windows(&display, &windows);
        let stream = sc::Stream::new(&filter, &cfg);

        // Delegate.
        let delegate = SpikeOutput::with(SpikeOutputInner {
            writer: writer.clone(),
        });

        // Serial dispatch queue for sample callbacks. `serial_with_ar_pool`
        // gives us autorelease pool drain semantics, matching the sc-record
        // example.
        let queue = dispatch::Queue::serial_with_ar_pool();
        stream
            .add_stream_output(delegate.as_ref(), sc::OutputType::Audio, Some(&queue))
            .map_err(|e| format!("add_stream_output(Audio) failed: {:?}", e))?;

        // Start. First call triggers the macOS Screen Recording prompt.
        stream
            .start()
            .await
            .map_err(|e| format!("stream.start failed: {:?}", e))?;
        println!("capturing system audio for 10 seconds...");

        tokio::time::sleep(Duration::from_secs(10)).await;

        stream
            .stop()
            .await
            .map_err(|e| format!("stream.stop failed: {:?}", e))?;

        // Finalize the WAV.
        let (frames, sr) = {
            let mut w = writer.lock().unwrap();
            w.finalize()?;
            (w.frames_written, w.sample_rate)
        };

        let meta = std::fs::metadata(path)?;
        println!(
            "wrote {} ({} bytes, {} frames at {} Hz)",
            path,
            meta.len(),
            frames,
            sr
        );
        Ok::<(), Box<dyn std::error::Error>>(())
    })?;

    Ok(())
}
