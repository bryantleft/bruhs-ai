# Meeting transcription quality, 2026-11-13

Whisper produced 19 words from digital silence in the control replay. Skipping inference only when every decoded sample is exactly zero removes those words. All speech text, timestamps and Me/Them labels are unchanged across three paired trials. New transcripts serialize unsupported confidence as `null` instead of claiming certainty.

This is local working-tree evidence for BNLE-42, not a published release. The default model and quiet-audio handling are unchanged.

## Evidence and release criteria

The frozen control comes from `a6f72da6`'s final-pass Whisper implementation. The control and candidate run in the same release binary against the same model and hashed recordings. The corpus contains 124 reference words across clean speech, technical names, British/Indian English voices, Spanish, French, noise, channel turns, overlap and two silent clips.

| Corpus/process metric | Control | Candidate |
| --- | ---: | ---: |
| Speech word errors / reference words | 62 / 124 | 62 / 124 |
| Speech WER | 0.500 | 0.500 |
| False words on silence | 19 | 0 |
| Me/Them duration error fraction | 0.8360 | 0.0933 |
| Median process wall time | 2.752 s | 1.653 s |
| Median process CPU time | 1.598 s | 1.192 s |
| Median process peak RSS | 325.7 MB | 295.7 MB |

The channel error fraction sums missed, false-alarm and confused speaker time, then divides by reference speaker time. Overlap counts both reference channels; silent-case false speech adds error time without adding reference time. The attribution improvement comes from eliminating false speech on silence. It does not demonstrate better speaker embeddings. English-only Base was deliberately forced to English on the Spanish/French stress cases; the aggregate WER is not a multilingual quality claim. Per-clip speech timings vary across runs and have overlapping ranges.

The measured release check requires unchanged speech text/timing/channel labels, zero silence words, and improvement over the paired control's silence count. Corpus WER and channel error must not exceed their paired baseline. Median wall/CPU/RSS must not exceed the measured control medians. These are same-corpus regression criteria, not universal latency or real-meeting quality guarantees.

Raw trials, the manifest, summary and hardware details are in `bench/transcription/results/2026-11-13-base-en/`. The machine is Mac14,6 with 12 physical cores and 32 GiB RAM, running macOS 26.3 arm64 and whisper-rs 0.12.0 with Metal. Measurements ran in a shared development session.

## Complete post-recording replay

`postprocess.json` records the real capture pipeline from mixing through model verification, final ASR, Me/Them assignment, durable transcript rewrite and completion receipt. Every saved segment matches the standalone candidate in text, timestamps and channel labels; every saved confidence value is `null`. Both silent clips persist empty transcripts and complete successfully. Original input hashes remain unchanged.

First-text and finalization timings are recorded per case. The first case includes cold model-cache verification; later cases reuse the verified cache. Process resource totals also include isolated model-copy and fixture setup. This is a new finalization baseline, not a before/after finalization comparison.

The replay does not start capture devices, measure device shutdown or live-preview latency, run summaries, download a model, or enable remote-speaker embeddings/OCR/cross-meeting linking. Speaker references cover known mic/system channels; scripted utterance extents include pauses. Synthetic voices and scripted overlap do not establish accuracy on real meetings.

## Reproduce

Run from the repository root with Rust 1.95 and an already-installed `ggml-base.en.bin` with SHA-256 `a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002` (the `WhisperModel::BaseEn` descriptor in `bruhs-models/src/models/whisper.rs`). The archived synthetic corpus preserves the exact recordings used in the saved results; regenerating with another OS/voice revision creates a new corpus and requires new paired results.

1. Extract the corpus:

   ```sh
   mkdir -p /tmp/bruhs-quality-corpus
   tar -xzf bench/transcription/fixtures/synthetic-v1.tar.gz -C /tmp/bruhs-quality-corpus
   ```

2. Build the standalone runner:

   ```sh
   cargo build --release --locked --manifest-path bench/transcription-eval/Cargo.toml
   ```

3. Set `model_path` to that installed model, then save three paired trials and a pipeline replay in one directory:

   ```sh
   mkdir -p /tmp/bruhs-quality-results
   for trial in 1 2 3; do
     python3 scripts/transcription_eval.py --binary bench/transcription-eval/target/release/bruhs-transcription-eval --model "$model_path" --manifest /tmp/bruhs-quality-corpus/manifest.json --variant legacy --output "/tmp/bruhs-quality-results/control-$trial.json"
     python3 scripts/transcription_eval.py --binary bench/transcription-eval/target/release/bruhs-transcription-eval --model "$model_path" --manifest /tmp/bruhs-quality-corpus/manifest.json --variant candidate --output "/tmp/bruhs-quality-results/candidate-$trial.json" --compare "/tmp/bruhs-quality-results/control-$trial.json"
   done
   python3 scripts/transcription_postprocess_eval.py --model "$model_path" --manifest /tmp/bruhs-quality-corpus/manifest.json --output /tmp/bruhs-quality-results/postprocess.json
   ```

4. Check the fresh outputs. The check calculates medians directly from the trial files; it does not consume `summary.json`:

   ```sh
   python3 scripts/transcription_release_check.py /tmp/bruhs-quality-results
   python3 -m unittest discover -s scripts -p test_transcription_eval.py
   ```

The same release-check command accepts `bench/transcription/results/2026-11-13-base-en` to verify the archived evidence. Timing failures on a busy machine require a new controlled paired measurement; the saved performance numbers are observations, not a fixed hardware-independent budget.

To evaluate a different model, use a separate output directory and matching model-language configuration. Compare candidates only against controls using that same model/corpus/scorer. No model default was changed here, and no alternative model is recommended by these results.

## Regression coverage

Tests cover digital silence with a nonexistent model, nonzero quiet audio still reaching inference, unsupported-confidence JSON/log/SQLite round trips, scoring and Unicode normalization, overlap attribution, changed evaluation inputs, independent process resource accounting, and rejected evidence with altered speech timing or unsupported confidence. Existing failure-recovery tests retain original recordings and suppress completion after a failed required stage.
