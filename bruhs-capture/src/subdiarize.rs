//! Sub-speaker diarization for the "them" channel.
//!
//! After the me/them split in `diarize.rs`, this module refines the "them"
//! segments into numbered sub-speakers (them_1, them_2, ...) using:
//!
//! 1. sherpa-rs's 3D-Speaker model to extract a per-segment embedding.
//! 2. L2-normalized embeddings so cosine similarity == dot product.
//! 3. Simple k-means (Lloyd's, 10 restarts, seed 42) with cosine distance.
//! 4. Silhouette score to auto-select k from 1..=max_speakers.
//! 5. Clusters labeled by first-appearance order so labels are deterministic.
//!
//! "me" segments pass through unchanged.

use bruhs_models::num::ToFloatLossy;
use bruhs_models::{ClusterCentroid, SpeakerId, TranscriptSegment};
#[cfg(target_os = "macos")]
use hound::{SampleFormat, WavReader};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
#[cfg(target_os = "macos")]
use rubato::{FftFixedIn, Resampler};
use std::collections::HashMap;
use std::path::Path;

use bruhs_models::OcrObservation;

// The speaker model was trained at 16 kHz.
#[cfg(target_os = "macos")]
const TARGET_SAMPLE_RATE: u32 = 16_000;
// Segments shorter than this (ms) produce very noisy embeddings; we still
// embed them but accept that clustering will absorb them into a neighbour.
#[cfg(target_os = "macos")]
const MIN_SEGMENT_MS: u32 = 250;
// K-means: number of random restarts. Best-inertia restart is kept.
const KMEANS_RESTARTS: usize = 10;
// K-means: fixed seed for reproducibility across runs.
const KMEANS_SEED: u64 = 42;
// K-means: maximum Lloyd iterations per restart.
const KMEANS_MAX_ITER: usize = 100;

// ─────────────────────────────────────────────────────────────────────────────
// Error type
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum SubdiarizeError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("hound: {0}")]
    Hound(#[from] hound::Error),
    #[error("resample: {0}")]
    Resample(String),
    #[error("sherpa: {0}")]
    Sherpa(String),
}

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Run sub-speaker diarization on the "them" portion of segments.
/// Rewrites `speaker_id` from `"them"` → `"them_1"`, `"them_2"`, … based on
/// embedding clusters. `"me"` segments are untouched.
///
/// Returns `(cluster_count, centroids)` where `centroids` contains one
/// L2-normalized centroid per cluster alongside its label and earliest
/// start_ms. Returns `Ok((0, vec![]))` if there are no "them" segments.
#[cfg(target_os = "macos")]
pub fn assign_subspeakers(
    segments: &mut [TranscriptSegment],
    sys_wav_path: &Path,
    speaker_model_path: &Path,
    max_speakers: usize,
) -> Result<(usize, Vec<ClusterCentroid>), SubdiarizeError> {
    // 1. Collect indices of "them" segments.
    let them_indices: Vec<usize> = segments
        .iter()
        .enumerate()
        .filter(|(_, s)| s.speaker_id.as_deref() == Some("them"))
        .map(|(i, _)| i)
        .collect();

    if them_indices.is_empty() {
        return Ok((0, Vec::new()));
    }

    // 2. Load sys.wav as 16-bit mono PCM, convert to f32, resample to 16 kHz.
    let mono_16k = load_wav_mono_16k(sys_wav_path)?;

    // 3. Initialise sherpa-rs embedding extractor.
    let model_str = speaker_model_path
        .to_str()
        .ok_or_else(|| SubdiarizeError::Sherpa("model path is not valid UTF-8".into()))?;
    let config = sherpa_rs::speaker_id::ExtractorConfig {
        model: model_str.into(),
        ..Default::default()
    };
    let mut extractor = sherpa_rs::speaker_id::EmbeddingExtractor::new(config)
        .map_err(|e| SubdiarizeError::Sherpa(format!("{e:?}")))?;

    // 4. Per-segment: slice samples → embed → L2-normalize.
    let mut embeddings: Vec<Vec<f32>> = Vec::with_capacity(them_indices.len());
    for &idx in &them_indices {
        let seg = &segments[idx];
        let start_ms = seg.start_ms;
        let end_ms = seg.end_ms;

        // Warn but don't skip very short segments — clustering absorbs them.
        if end_ms.saturating_sub(start_ms) < MIN_SEGMENT_MS {
            tracing::debug!(
                segment_id = %seg.id,
                duration_ms = end_ms.saturating_sub(start_ms),
                "segment below min duration; embedding may be noisy"
            );
        }

        let chunk = slice_samples(&mono_16k, start_ms, end_ms, TARGET_SAMPLE_RATE);
        let raw = extractor
            .compute_speaker_embedding(chunk, TARGET_SAMPLE_RATE)
            .map_err(|e| SubdiarizeError::Sherpa(format!("{e:?}")))?;
        embeddings.push(l2_normalize(&raw));
    }

    // 5. Pick best k in 1..=max_speakers by silhouette score.
    let k_max = max_speakers.max(1).min(embeddings.len());
    let best_k = choose_k(&embeddings, k_max);

    // 6. Final clustering at chosen k.
    let assignments = kmeans(&embeddings, best_k, KMEANS_SEED);

    // 7. Build first-appearance label map: cluster id → 1-based display index.
    let label_map = first_appearance_labels(&assignments, best_k);

    // 8. Rewrite speaker_id on segments via the typed SpeakerId constructor.
    for (local_i, &seg_idx) in them_indices.iter().enumerate() {
        let cluster = assignments[local_i];
        let label = label_map[cluster];
        segments[seg_idx].speaker_id =
            Some(SpeakerId::Cluster(u32::try_from(label).unwrap_or(u32::MAX)).to_raw());
    }

    // 9. Compute per-cluster centroids (L2-normalized mean of member embeddings)
    //    and earliest start_ms, in first-appearance label order.
    let centroids = build_centroids(
        &embeddings,
        &assignments,
        best_k,
        &label_map,
        &them_indices,
        segments,
    );

    // 10. Write per-segment cosine similarity to the assigned cluster's centroid.
    //     This gives the UI a confidence signal to show on uncertain segments.
    for (local_i, &seg_idx) in them_indices.iter().enumerate() {
        let cluster = assignments[local_i];
        let centroid = &centroids[label_map[cluster] - 1]; // label is 1-based, vec is 0-based
        let sim = cosine_similarity(&embeddings[local_i], &centroid.embedding);
        segments[seg_idx].speaker_confidence = Some(sim);
    }

    Ok((best_k, centroids))
}

/// Stub for non-macOS builds so the module still compiles.
#[cfg(not(target_os = "macos"))]
pub fn assign_subspeakers(
    _segments: &mut [TranscriptSegment],
    _sys_wav_path: &Path,
    _speaker_model_path: &Path,
    _max_speakers: usize,
) -> Result<(usize, Vec<ClusterCentroid>), SubdiarizeError> {
    Ok((0, Vec::new()))
}

// ─────────────────────────────────────────────────────────────────────────────
// WAV loading + resampling helpers (macOS-only: used only by assign_subspeakers)
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn load_wav_mono_16k(path: &Path) -> Result<Vec<f32>, SubdiarizeError> {
    let mut reader = WavReader::open(path)?;
    let spec = reader.spec();
    if spec.bits_per_sample != 16 || spec.sample_format != SampleFormat::Int {
        return Err(SubdiarizeError::Hound(hound::Error::Unsupported));
    }
    let channels = spec.channels as usize;
    let mut mono: Vec<f32> = Vec::with_capacity(reader.len() as usize / channels.max(1));
    let mut chan_buf = vec![0i32; channels];
    let mut chan_idx = 0usize;
    for sample in reader.samples::<i16>() {
        let s = sample?;
        chan_buf[chan_idx] = i32::from(s);
        chan_idx += 1;
        if chan_idx == channels {
            let sum: i32 = chan_buf.iter().copied().sum();
            mono.push(sum.to_f32_lossy() / channels.to_f32_lossy() / f32::from(i16::MAX));
            chan_idx = 0;
        }
    }

    if spec.sample_rate == TARGET_SAMPLE_RATE {
        return Ok(mono);
    }
    resample_mono(mono, spec.sample_rate, TARGET_SAMPLE_RATE)
}

#[cfg(target_os = "macos")]
fn resample_mono(
    input: Vec<f32>,
    in_rate: u32,
    out_rate: u32,
) -> Result<Vec<f32>, SubdiarizeError> {
    let chunk_size = (in_rate as usize / 4).max(256).min(input.len().max(256));
    let mut resampler =
        FftFixedIn::<f32>::new(in_rate as usize, out_rate as usize, chunk_size, 2, 1)
            .map_err(|e| SubdiarizeError::Resample(format!("{e}")))?;

    let exact_out = crate::mix::resampled_len(input.len(), in_rate, out_rate);
    let mut out: Vec<f32> = Vec::with_capacity(exact_out + chunk_size);
    let mut cursor = 0;
    while cursor < input.len() {
        let end = (cursor + chunk_size).min(input.len());
        let mut chunk: Vec<f32> = input[cursor..end].to_vec();
        if chunk.len() < chunk_size {
            chunk.resize(chunk_size, 0.0);
        }
        let frames = vec![chunk];
        let resampled = resampler
            .process(&frames, None)
            .map_err(|e| SubdiarizeError::Resample(format!("{e}")))?;
        out.extend_from_slice(&resampled[0]);
        cursor += chunk_size;
    }
    out.truncate(exact_out);
    Ok(out)
}

/// Extract a contiguous slice of mono samples for the half-open window
/// [start_ms, end_ms) at the given sample rate. Clamps to valid range.
fn slice_samples(mono: &[f32], start_ms: u32, end_ms: u32, sample_rate: u32) -> Vec<f32> {
    let at = |ms: u32| {
        usize::try_from(u64::from(ms) * u64::from(sample_rate) / 1000).unwrap_or(usize::MAX)
    };
    let start = at(start_ms);
    let end = at(end_ms);
    let lo = start.min(mono.len());
    let hi = end.min(mono.len());
    mono[lo..hi].to_vec()
}

// ─────────────────────────────────────────────────────────────────────────────
// Pure clustering helpers (tested without sherpa or hound)
// ─────────────────────────────────────────────────────────────────────────────

/// L2-normalize a vector. Returns a zero vector if the norm is 0.
pub fn l2_normalize(v: &[f32]) -> Vec<f32> {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm < 1e-9 {
        return vec![0.0; v.len()];
    }
    v.iter().map(|x| x / norm).collect()
}

/// Dot product of two L2-normalized vectors, equivalent to cosine similarity.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Lloyd's k-means with cosine distance (maximise dot product of L2-normalized
/// embeddings). Runs `KMEANS_RESTARTS` random initializations; returns the
/// assignment from the restart with the best (lowest) inertia.
///
/// `embeddings` must already be L2-normalized. `k` must be ≥ 1 and ≤ len.
pub fn kmeans(embeddings: &[Vec<f32>], k: usize, seed: u64) -> Vec<usize> {
    assert!(!embeddings.is_empty(), "kmeans: empty input");
    assert!(
        k >= 1 && k <= embeddings.len(),
        "kmeans: k={k} out of range"
    );

    if k == 1 {
        return vec![0; embeddings.len()];
    }

    let mut rng = StdRng::seed_from_u64(seed);
    let mut best_assignments: Vec<usize> = vec![0; embeddings.len()];
    let mut best_inertia = f32::NEG_INFINITY;

    for _ in 0..KMEANS_RESTARTS {
        // Random init: pick k distinct seed indices.
        let mut center_indices: Vec<usize> = Vec::with_capacity(k);
        while center_indices.len() < k {
            let idx = rng.random_range(0..embeddings.len());
            if !center_indices.contains(&idx) {
                center_indices.push(idx);
            }
        }
        let mut centroids: Vec<Vec<f32>> = center_indices
            .iter()
            .map(|&i| embeddings[i].clone())
            .collect();

        let mut assignments = vec![0usize; embeddings.len()];
        for _ in 0..KMEANS_MAX_ITER {
            // Assignment step: each embedding → nearest centroid (max cosine sim).
            let mut changed = false;
            for (i, emb) in embeddings.iter().enumerate() {
                let best = (0..k)
                    .max_by(|&a, &b| {
                        cosine_similarity(emb, &centroids[a])
                            .partial_cmp(&cosine_similarity(emb, &centroids[b]))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .unwrap_or(0);
                if assignments[i] != best {
                    assignments[i] = best;
                    changed = true;
                }
            }

            // Update step: centroid = L2-normalized mean of assigned embeddings.
            for (c, centroid) in centroids.iter_mut().enumerate() {
                let members: Vec<&Vec<f32>> = assignments
                    .iter()
                    .zip(embeddings.iter())
                    .filter(|(&a, _)| a == c)
                    .map(|(_, e)| e)
                    .collect();
                if members.is_empty() {
                    // Empty cluster: re-seed to a random embedding.
                    *centroid = embeddings[rng.random_range(0..embeddings.len())].clone();
                } else {
                    let dim = members[0].len();
                    let mut mean = vec![0.0f32; dim];
                    for emb in &members {
                        for (m, &x) in mean.iter_mut().zip(emb.iter()) {
                            *m += x;
                        }
                    }
                    *centroid = l2_normalize(&mean);
                }
            }

            if !changed {
                break;
            }
        }

        // Compute inertia = sum of cosine similarities to assigned centroid
        // (higher is better because we maximise cosine similarity).
        let inertia: f32 = embeddings
            .iter()
            .zip(assignments.iter())
            .map(|(e, &c)| cosine_similarity(e, &centroids[c]))
            .sum();

        if inertia > best_inertia {
            best_inertia = inertia;
            best_assignments = assignments;
        }
    }

    best_assignments
}

/// Mean silhouette coefficient for the given cluster assignments.
///
/// For k=1, returns 0.0 (undefined; we use 0 as a neutral baseline so k>1
/// only wins when it genuinely improves separation).
pub fn silhouette(embeddings: &[Vec<f32>], assignments: &[usize], k: usize) -> f32 {
    if k <= 1 || embeddings.len() < 2 {
        return 0.0;
    }

    // Cosine *distance* = 1 - cosine similarity (both are L2-normalized).
    let dist = |a: &[f32], b: &[f32]| -> f32 { 1.0 - cosine_similarity(a, b) };

    let n = embeddings.len();
    let mut total = 0.0f32;
    let mut counted = 0u32;

    for i in 0..n {
        let ci = assignments[i];

        // a(i): mean distance to all other points in the same cluster.
        let same: Vec<f32> = (0..n)
            .filter(|&j| j != i && assignments[j] == ci)
            .map(|j| dist(&embeddings[i], &embeddings[j]))
            .collect();
        if same.is_empty() {
            // Singleton cluster — silhouette is 0 by convention.
            counted += 1;
            continue;
        }
        let a = same.iter().sum::<f32>() / same.len().to_f32_lossy();

        // b(i): mean distance to all points in the nearest OTHER cluster.
        let b = (0..k)
            .filter(|&c| c != ci)
            .map(|c| {
                let others: Vec<f32> = (0..n)
                    .filter(|&j| assignments[j] == c)
                    .map(|j| dist(&embeddings[i], &embeddings[j]))
                    .collect();
                if others.is_empty() {
                    f32::INFINITY
                } else {
                    others.iter().sum::<f32>() / others.len().to_f32_lossy()
                }
            })
            .fold(f32::INFINITY, f32::min);

        if b.is_finite() {
            let s = (b - a) / a.max(b);
            total += s;
            counted += 1;
        }
    }

    if counted == 0 {
        0.0
    } else {
        total / counted.to_f32_lossy()
    }
}

/// Try k=1..=k_max; return the k with the highest silhouette score.
/// k=1 baseline is 0.0; only k>1 can beat it when clusters are well-separated.
#[cfg(target_os = "macos")]
fn choose_k(embeddings: &[Vec<f32>], k_max: usize) -> usize {
    if k_max == 1 || embeddings.len() == 1 {
        return 1;
    }

    let mut best_k = 1;
    let mut best_score = 0.0f32;

    for k in 1..=k_max.min(embeddings.len()) {
        let assignments = kmeans(embeddings, k, KMEANS_SEED);
        let score = silhouette(embeddings, &assignments, k);
        if score > best_score {
            best_score = score;
            best_k = k;
        }
    }

    best_k
}

/// Build a mapping from cluster id → 1-based display label ordered by first
/// appearance in the `assignments` slice.
///
/// Example: assignments = [1, 0, 1, 0] → map = {1 → 1, 0 → 2}
/// (cluster 1 appears first, so it becomes label 1; cluster 0 is label 2).
pub fn first_appearance_labels(assignments: &[usize], k: usize) -> Vec<usize> {
    let mut label_map = vec![0usize; k];
    let mut seen = vec![false; k];
    let mut next_label = 1usize;
    for &cluster in assignments {
        if cluster < k && !seen[cluster] {
            label_map[cluster] = next_label;
            seen[cluster] = true;
            next_label += 1;
        }
    }
    // Any cluster never assigned (k > distinct clusters): give it a label anyway.
    for c in 0..k {
        if !seen[c] {
            label_map[c] = next_label;
            next_label += 1;
        }
    }
    label_map
}

/// Build one [`ClusterCentroid`] per cluster, ordered by first-appearance label
/// (i.e. `them_1`, `them_2`, …).
///
/// `label_map[cluster_id] = 1-based display label` — we invert it to iterate
/// in label order. The centroid embedding is the L2-normalized mean of all
/// member embeddings. `first_seen_ms` is the minimum `start_ms` among segments
/// belonging to that cluster.
fn build_centroids(
    embeddings: &[Vec<f32>],
    assignments: &[usize],
    k: usize,
    label_map: &[usize],
    them_indices: &[usize],
    segments: &[TranscriptSegment],
) -> Vec<ClusterCentroid> {
    // Invert label_map: label (1-based) → cluster_id.
    let mut label_to_cluster = vec![0usize; k + 1]; // index 0 unused
    for (cluster_id, &label) in label_map.iter().enumerate() {
        if label <= k {
            label_to_cluster[label] = cluster_id;
        }
    }

    (1..=k)
        .map(|label| {
            let cluster_id = label_to_cluster[label];

            // Collect member embeddings and their segment start_ms.
            let dim = embeddings.first().map_or(0, |e| e.len());
            let mut mean = vec![0.0f32; dim];
            let mut first_seen_ms = u32::MAX;
            let mut count = 0usize;

            for (local_i, &seg_idx) in them_indices.iter().enumerate() {
                if assignments[local_i] == cluster_id {
                    let emb = &embeddings[local_i];
                    for (m, &x) in mean.iter_mut().zip(emb.iter()) {
                        *m += x;
                    }
                    let start = segments[seg_idx].start_ms;
                    if start < first_seen_ms {
                        first_seen_ms = start;
                    }
                    count += 1;
                }
            }

            // L2-normalize the mean (or zero-vector if cluster is empty).
            if count > 0 {
                for x in mean.iter_mut() {
                    *x /= count.to_f32_lossy();
                }
            }
            let centroid_embedding = l2_normalize(&mean);

            ClusterCentroid {
                cluster_label: SpeakerId::Cluster(u32::try_from(label).unwrap_or(u32::MAX))
                    .to_raw(),
                embedding: centroid_embedding,
                first_seen_ms: if first_seen_ms == u32::MAX {
                    0
                } else {
                    first_seen_ms
                },
            }
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Phase Vision: OCR name correlation
// ─────────────────────────────────────────────────────────────────────────────

/// Minimum number of OCR votes before suggesting a name for review.
const MIN_NAME_VOTES: u32 = 5;

/// ±ms window around each segment to search for OCR observations.
const OCR_WINDOW_MS: u32 = 2_000;

/// Unverified screen-name hint, never a speaker identity or voice enrollment.
#[derive(Debug, PartialEq, Eq)]
pub struct OcrNameHint {
    pub cluster_id: String,
    pub candidate_name: String,
}

/// Correlate screen names for possible review without changing transcript IDs.
/// A persistent pinned tile can win every vote while somebody else speaks.
/// OCR has no active-speaker evidence, so callers must not apply these hints
/// automatically or use them to link voice profiles.
pub fn ocr_name_hints(
    segments: &[TranscriptSegment],
    observations: &[OcrObservation],
) -> Vec<OcrNameHint> {
    let mut hints = Vec::new();
    if observations.is_empty() {
        return hints;
    }

    // Collect distinct "them_N" cluster labels.
    let cluster_labels: Vec<String> = {
        let mut labels: Vec<String> = segments
            .iter()
            .filter_map(|s| s.speaker_id.as_deref())
            .filter(|id| id.starts_with("them_"))
            .map(str::to_owned)
            .collect();
        labels.sort_unstable();
        labels.dedup();
        labels
    };

    for label in &cluster_labels {
        // Gather time windows for this cluster.
        let mut name_counts: HashMap<String, u32> = HashMap::new();

        for seg in segments
            .iter()
            .filter(|s| s.speaker_id.as_deref() == Some(label.as_str()))
        {
            let win_start = seg.start_ms.saturating_sub(OCR_WINDOW_MS);
            let win_end = seg.end_ms.saturating_add(OCR_WINDOW_MS);
            for obs in observations
                .iter()
                .filter(|o| o.timestamp_ms >= win_start && o.timestamp_ms <= win_end)
            {
                *name_counts.entry(obs.text.clone()).or_insert(0) += 1;
            }
        }

        if let Some((winner, _count)) = pick_dominant(&name_counts) {
            hints.push(OcrNameHint {
                cluster_id: label.clone(),
                candidate_name: winner,
            });
        }
    }
    hints
}

/// Return the dominant name from a vote map if:
/// - the winner has `≥ MIN_NAME_VOTES` total votes, AND
/// - the winner has `≥ 1.5×` the runner-up count.
///
/// Returns `None` if the map is empty, the threshold isn't met, or there
/// aren't enough votes.
pub fn pick_dominant(counts: &HashMap<String, u32>) -> Option<(String, u32)> {
    let mut sorted: Vec<(&String, u32)> = counts.iter().map(|(k, &v)| (k, v)).collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));

    let (top_name, top_count) = sorted.first()?;
    if *top_count < MIN_NAME_VOTES {
        return None;
    }
    let runner_up_count = sorted.get(1).map_or(0, |(_, c)| *c);
    let margin = top_count.to_f32_lossy() / (runner_up_count.to_f32_lossy() + 1.0);
    if margin >= 1.5 {
        Some(((*top_name).clone(), *top_count))
    } else {
        None
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_segment(
        id: &str,
        start_ms: u32,
        end_ms: u32,
        speaker: Option<&str>,
    ) -> TranscriptSegment {
        TranscriptSegment {
            id: id.into(),
            meeting_id: "m-1".into(),
            speaker_id: speaker.map(str::to_owned),
            start_ms,
            end_ms,
            text: "x".into(),
            confidence: 1.0,
            partial: false,
            speaker_confidence: None,
        }
    }

    // ── k-means ──────────────────────────────────────────────────────────────

    #[test]
    fn kmeans_two_clear_clusters() {
        // Two tight clouds in 2D: cluster A near [1,0], cluster B near [0,1].
        let cluster_a = [vec![1.0_f32, 0.0], vec![1.0, 0.1], vec![0.9, 0.0]];
        let cluster_b = [vec![0.0_f32, 1.0], vec![0.0, 0.9], vec![0.1, 1.0]];
        let all: Vec<Vec<f32>> = cluster_a.iter().chain(cluster_b.iter()).cloned().collect();
        let normalized: Vec<Vec<f32>> = all.iter().map(|v| l2_normalize(v)).collect();
        let assignments = kmeans(&normalized, 2, 42);

        // All A members in the same cluster, all B members in the same cluster,
        // and the two groups are distinct.
        assert_eq!(assignments[0], assignments[1]);
        assert_eq!(assignments[1], assignments[2]);
        assert_eq!(assignments[3], assignments[4]);
        assert_eq!(assignments[4], assignments[5]);
        assert_ne!(assignments[0], assignments[3]);
    }

    #[test]
    fn kmeans_k1_returns_all_zeros() {
        let embeddings = [vec![1.0_f32, 0.0], vec![0.0, 1.0], vec![-1.0, 0.0]];
        let normalized: Vec<Vec<f32>> = embeddings.iter().map(|v| l2_normalize(v)).collect();
        let assignments = kmeans(&normalized, 1, 42);
        assert!(assignments.iter().all(|&a| a == 0));
    }

    #[test]
    fn kmeans_k_equals_n_all_unique() {
        let embeddings = [vec![1.0_f32, 0.0], vec![0.0, 1.0], vec![0.0, -1.0]];
        let normalized: Vec<Vec<f32>> = embeddings.iter().map(|v| l2_normalize(v)).collect();
        let assignments = kmeans(&normalized, 3, 42);
        // All different.
        assert_ne!(assignments[0], assignments[1]);
        assert_ne!(assignments[0], assignments[2]);
        assert_ne!(assignments[1], assignments[2]);
    }

    // ── silhouette ───────────────────────────────────────────────────────────

    #[test]
    fn silhouette_higher_for_well_separated() {
        // Two tight, well-separated clouds.
        let cloud_a = [
            vec![1.0_f32, 0.01, 0.0],
            vec![1.0, -0.01, 0.0],
            vec![0.99, 0.0, 0.01],
        ];
        let cloud_b = [
            vec![0.0_f32, 1.0, 0.01],
            vec![0.0, 1.0, -0.01],
            vec![0.01, 0.99, 0.0],
        ];
        let all: Vec<Vec<f32>> = cloud_a.iter().chain(cloud_b.iter()).cloned().collect();
        let normalized: Vec<Vec<f32>> = all.iter().map(|v| l2_normalize(v)).collect();

        // k=2 should have clearly better silhouette than k=1 (which is 0.0).
        let assignments_2 = kmeans(&normalized, 2, 42);
        let score_2 = silhouette(&normalized, &assignments_2, 2);
        let score_1 = silhouette(&normalized, &[0; 6], 1);

        assert!(
            score_2 > score_1,
            "expected silhouette(k=2)={score_2:.4} > silhouette(k=1)={score_1:.4}"
        );
        assert!(
            score_2 > 0.5,
            "expected high silhouette for well-separated clusters, got {score_2:.4}"
        );
    }

    #[test]
    fn silhouette_k1_is_zero() {
        let embeddings = [vec![1.0_f32, 0.0], vec![0.0, 1.0]];
        let normalized: Vec<Vec<f32>> = embeddings.iter().map(|v| l2_normalize(v)).collect();
        assert_eq!(silhouette(&normalized, &[0; 2], 1), 0.0);
    }

    // ── first-appearance labels ───────────────────────────────────────────────

    #[test]
    fn cluster_labels_by_first_appearance() {
        // assignments [1, 0, 1, 0]: cluster 1 appears first → label 1; cluster 0 → label 2.
        let assignments = vec![1, 0, 1, 0];
        let labels = first_appearance_labels(&assignments, 2);
        // cluster 1 → label 1, cluster 0 → label 2.
        assert_eq!(labels[1], 1);
        assert_eq!(labels[0], 2);
    }

    #[test]
    fn cluster_labels_by_first_appearance_sequential() {
        // assignments [0, 1, 2]: cluster 0 appears first → 1, then 1 → 2, then 2 → 3.
        let assignments = vec![0, 1, 2];
        let labels = first_appearance_labels(&assignments, 3);
        assert_eq!(labels[0], 1);
        assert_eq!(labels[1], 2);
        assert_eq!(labels[2], 3);
    }

    // ── segment label rewriting ───────────────────────────────────────────────

    #[test]
    fn assign_subspeakers_skips_me_segments() {
        // Build segments: two "me" and two "them" interleaved.
        // We test the rewriting logic in isolation by calling the sub-functions
        // directly (no sherpa, no WAV I/O).
        let mut segs = [
            make_segment("s1", 0, 500, Some("me")),
            make_segment("s2", 500, 1000, Some("them")),
            make_segment("s3", 1000, 1500, Some("me")),
            make_segment("s4", 1500, 2000, Some("them")),
        ];

        // Synthetic embeddings for the two "them" segments.
        let them_indices = [1usize, 3];
        let embeddings = [l2_normalize(&[1.0_f32, 0.0]), l2_normalize(&[1.0, 0.1])];
        let assignments = kmeans(&embeddings, 1, 42); // k=1: both same cluster
        let label_map = first_appearance_labels(&assignments, 1);

        for (local_i, &seg_idx) in them_indices.iter().enumerate() {
            let cluster = assignments[local_i];
            let label = label_map[cluster];
            segs[seg_idx].speaker_id = Some(format!("them_{label}"));
        }

        // "me" segments untouched.
        assert_eq!(segs[0].speaker_id.as_deref(), Some("me"));
        assert_eq!(segs[2].speaker_id.as_deref(), Some("me"));
        // "them" segments rewritten to them_1.
        assert_eq!(segs[1].speaker_id.as_deref(), Some("them_1"));
        assert_eq!(segs[3].speaker_id.as_deref(), Some("them_1"));
    }

    #[test]
    fn two_speakers_split_correctly_via_label_rewrite() {
        // Two clearly separated embeddings → k=2 → labels them_1 / them_2.
        let mut segs = [
            make_segment("s1", 0, 500, Some("them")),
            make_segment("s2", 500, 1000, Some("them")),
        ];
        let them_indices = [0usize, 1];
        let embeddings = [l2_normalize(&[1.0_f32, 0.0]), l2_normalize(&[0.0, 1.0])];
        // Force k=2 (clearly separated → silhouette selects this in the real path).
        let assignments = kmeans(&embeddings, 2, 42);
        let label_map = first_appearance_labels(&assignments, 2);

        for (local_i, &seg_idx) in them_indices.iter().enumerate() {
            let cluster = assignments[local_i];
            let label = label_map[cluster];
            segs[seg_idx].speaker_id = Some(format!("them_{label}"));
        }

        // Both should have distinct sub-speaker labels.
        let lbl0 = segs[0].speaker_id.clone().unwrap();
        let lbl1 = segs[1].speaker_id.clone().unwrap();
        assert!(lbl0.starts_with("them_"), "expected them_N got {lbl0}");
        assert!(lbl1.starts_with("them_"), "expected them_N got {lbl1}");
        assert_ne!(
            lbl0, lbl1,
            "two clearly separate speakers should get different labels"
        );
    }

    // ── slice_samples ─────────────────────────────────────────────────────────

    #[test]
    fn slice_samples_extracts_correct_window() {
        // 1 second of 16kHz audio: index 0..16000.
        let mono: Vec<f32> = (0..16_000u16).map(f32::from).collect();
        // Slice 500ms-1000ms → samples 8000..16000.
        let chunk = slice_samples(&mono, 500, 1000, 16_000);
        assert_eq!(chunk.len(), 8000);
        assert!((chunk[0] - 8000.0).abs() < 1e-3);
    }

    #[test]
    fn slice_samples_clamps_to_end() {
        let mono: Vec<f32> = vec![0.0; 100];
        // Ask for beyond the end → should clamp, not panic.
        let chunk = slice_samples(&mono, 0, 60_000, 16_000);
        assert_eq!(chunk.len(), 100);
    }

    // ── build_centroids ───────────────────────────────────────────────────────

    #[test]
    fn centroids_have_unit_l2_norm() {
        // Two clear clusters in 2D; build_centroids should return L2-normalized
        // embeddings with norm ≈ 1.0.
        let embeddings = vec![
            l2_normalize(&[1.0_f32, 0.0]),
            l2_normalize(&[1.0, 0.05]),
            l2_normalize(&[0.0, 1.0]),
            l2_normalize(&[0.05, 1.0]),
        ];
        let assignments = kmeans(&embeddings, 2, 42);
        let label_map = first_appearance_labels(&assignments, 2);

        let segs = [
            make_segment("s1", 0, 500, Some("them")),
            make_segment("s2", 500, 1000, Some("them")),
            make_segment("s3", 1000, 1500, Some("them")),
            make_segment("s4", 1500, 2000, Some("them")),
        ];
        let them_indices: Vec<usize> = (0..4).collect();

        let centroids = build_centroids(
            &embeddings,
            &assignments,
            2,
            &label_map,
            &them_indices,
            &segs,
        );

        assert_eq!(centroids.len(), 2);
        for c in &centroids {
            let norm: f32 = c.embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
            assert!(
                (norm - 1.0).abs() < 1e-5,
                "centroid {} has L2 norm {norm}, expected ≈ 1.0",
                c.cluster_label
            );
        }
    }

    #[test]
    fn centroids_first_seen_ms_is_minimum() {
        let embeddings = vec![l2_normalize(&[1.0_f32, 0.0]), l2_normalize(&[1.0, 0.05])];
        let assignments = vec![0usize, 0]; // both in cluster 0
        let label_map = first_appearance_labels(&assignments, 1);

        let segs = [
            make_segment("s1", 300, 800, Some("them")),
            make_segment("s2", 100, 600, Some("them")),
        ];
        let them_indices: Vec<usize> = vec![0, 1];

        let centroids = build_centroids(
            &embeddings,
            &assignments,
            1,
            &label_map,
            &them_indices,
            &segs,
        );
        assert_eq!(centroids.len(), 1);
        assert_eq!(
            centroids[0].first_seen_ms, 100,
            "first_seen_ms should be the minimum start_ms"
        );
    }

    // ── l2_normalize ─────────────────────────────────────────────────────────

    #[test]
    fn l2_normalize_unit_vector() {
        let v = vec![3.0_f32, 4.0];
        let n = l2_normalize(&v);
        let norm: f32 = n.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-6, "norm={norm}");
    }

    #[test]
    fn l2_normalize_zero_vector() {
        let v = vec![0.0_f32, 0.0, 0.0];
        let n = l2_normalize(&v);
        assert!(n.iter().all(|&x| x == 0.0));
    }

    // ── pick_dominant ─────────────────────────────────────────────────────────

    #[test]
    fn pick_dominant_clear_winner() {
        let mut counts = std::collections::HashMap::new();
        counts.insert("Alice".to_string(), 10u32);
        counts.insert("Bob".to_string(), 3u32);
        let result = pick_dominant(&counts);
        assert_eq!(result, Some(("Alice".to_string(), 10)));
    }

    #[test]
    fn pick_dominant_not_enough_votes() {
        let mut counts = std::collections::HashMap::new();
        counts.insert("Alice".to_string(), 4u32); // below MIN_NAME_VOTES = 5
        let result = pick_dominant(&counts);
        assert_eq!(result, None);
    }

    #[test]
    fn pick_dominant_too_close() {
        let mut counts = std::collections::HashMap::new();
        counts.insert("Alice".to_string(), 6u32);
        counts.insert("Bob".to_string(), 5u32); // Alice/Bob ratio < 1.5
        let result = pick_dominant(&counts);
        assert_eq!(result, None);
    }

    #[test]
    fn pick_dominant_empty_map() {
        let counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        assert_eq!(pick_dominant(&counts), None);
    }

    // ── unverified OCR hints ──────────────────────────────────────────────────

    use bruhs_models::{OcrBoundingBox, OcrObservation};

    fn make_ocr_obs(text: &str, timestamp_ms: u32) -> OcrObservation {
        OcrObservation {
            text: text.to_string(),
            bbox: OcrBoundingBox {
                x: 0.1,
                y: 0.1,
                w: 0.2,
                h: 0.05,
            },
            confidence: 0.95,
            timestamp_ms,
        }
    }

    #[test]
    fn dominant_screen_name_is_only_a_hint_and_preserves_cluster_identity() {
        let segs = [
            make_segment("s1", 1000, 2000, Some("them_1")),
            make_segment("s2", 3000, 4000, Some("them_1")),
        ];
        // 8 observations of "Alice" in window of s1 and s2 (±2000ms)
        let observations: Vec<OcrObservation> = (0..8)
            .map(|i| make_ocr_obs("Alice", 1500 + i * 200))
            .collect();
        let original = serde_json::to_value(&segs).unwrap();
        assert_eq!(
            ocr_name_hints(&segs, &observations),
            vec![OcrNameHint {
                cluster_id: "them_1".into(),
                candidate_name: "Alice".into(),
            }]
        );
        assert_eq!(serde_json::to_value(&segs).unwrap(), original);
    }

    #[test]
    fn assign_names_leaves_me_untouched() {
        let segs = [make_segment("s1", 1000, 2000, Some("me"))];
        let observations = vec![make_ocr_obs("Alice", 1500)];
        assert!(ocr_name_hints(&segs, &observations).is_empty());
        assert_eq!(segs[0].speaker_id.as_deref(), Some("me"));
    }

    #[test]
    fn assign_names_no_change_when_no_winner() {
        let segs = [make_segment("s1", 1000, 2000, Some("them_1"))];
        // Only 2 votes — below threshold
        let observations = vec![make_ocr_obs("Alice", 1500), make_ocr_obs("Alice", 1600)];
        assert!(ocr_name_hints(&segs, &observations).is_empty());
        assert_eq!(segs[0].speaker_id.as_deref(), Some("them_1"));
    }

    #[test]
    fn assign_names_no_change_when_observations_empty() {
        let segs = [make_segment("s1", 1000, 2000, Some("them_1"))];
        assert!(ocr_name_hints(&segs, &[]).is_empty());
        assert_eq!(segs[0].speaker_id.as_deref(), Some("them_1"));
    }

    #[test]
    fn pinned_name_cannot_merge_distinct_voices_or_overwrite_a_manual_label() {
        let segs = [
            make_segment("s1", 1000, 2000, Some("them_1")),
            make_segment("s2", 1000, 2000, Some("them_2")),
            make_segment("s3", 1000, 2000, Some("name:Manually chosen")),
        ];
        let observations: Vec<_> = (0..20)
            .map(|i| make_ocr_obs("Pinned participant", 1000 + i * 50))
            .collect();
        let hints = ocr_name_hints(&segs, &observations);
        assert_eq!(hints.len(), 2);
        assert_ne!(hints[0].cluster_id, hints[1].cluster_id);
        assert_eq!(hints[0].candidate_name, hints[1].candidate_name);
        assert_eq!(segs[2].speaker_id.as_deref(), Some("name:Manually chosen"));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("segments.jsonl");
        crate::transcribe::segments_log::write_all(&path, &segs).unwrap();
        let restored = crate::transcribe::segments_log::load_snapshot(&path).unwrap();
        assert_eq!(
            serde_json::to_value(restored).unwrap(),
            serde_json::to_value(segs).unwrap()
        );
    }
}
