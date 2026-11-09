//! Spike: prove `sherpa-rs` loads a 3D-Speaker ONNX model and produces a
//! non-zero speaker embedding from a synthetic 16 kHz mono sine wave.
//!
//! Run from `bruhs-capture/`: `cargo run --example sherpa_spike`
//!
//! Output: `embedding len = <N>` and `embedding L2 norm = <nonzero>`.
//!
//! Model is cached at `/tmp/3dspeaker_speech_campplus_sv_en_voxceleb_16k.onnx`
//! on first run (~28 MB). Pinned to the sherpa-onnx `speaker-recongition-models`
//! release (sic — upstream typo in the tag name).

use bruhs_models::num::{ToFloatLossy, ToIntSaturating};

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("This spike is macOS-only.");
    std::process::exit(1);
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::path::Path;

    // Pinned model URL — 3D-Speaker CAM++ English VoxCeleb, 16 kHz, ~28 MB.
    // Note: upstream release tag has a typo ("recongition", not "recognition").
    const MODEL_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_campplus_sv_en_voxceleb_16k.onnx";
    const MODEL_PATH: &str = "/tmp/3dspeaker_speech_campplus_sv_en_voxceleb_16k.onnx";

    // ---- 1. Ensure the model file is present. ---------------------------
    if !Path::new(MODEL_PATH).exists() {
        println!("downloading model -> {MODEL_PATH}");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let bytes = rt.block_on(async {
            let resp = reqwest::Client::new().get(MODEL_URL).send().await?;
            resp.error_for_status()?.bytes().await
        })?;
        std::fs::write(MODEL_PATH, &bytes)?;
        println!("downloaded {} bytes", bytes.len());
    } else {
        println!("model already cached at {MODEL_PATH}");
    }

    // ---- 2. Build a 2 s, 16 kHz mono sine at 440 Hz, amplitude 0.3. -----
    let sample_rate: u32 = 16_000;
    let duration_s: f32 = 2.0;
    let n = (sample_rate.to_f32_lossy() * duration_s).to_usize_saturating();
    let samples: Vec<f32> = (0..n)
        .map(|i| {
            (i.to_f32_lossy() / sample_rate.to_f32_lossy() * 440.0 * std::f32::consts::TAU).sin()
                * 0.3
        })
        .collect();
    println!(
        "generated {} samples ({:.2}s @ {} Hz)",
        samples.len(),
        duration_s,
        sample_rate
    );

    // ---- 3. Load sherpa-rs's speaker embedding extractor. ---------------
    println!("loading sherpa-rs model");
    let config = sherpa_rs::speaker_id::ExtractorConfig {
        model: MODEL_PATH.into(),
        ..Default::default()
    };
    let mut extractor = sherpa_rs::speaker_id::EmbeddingExtractor::new(config)
        .map_err(|e| format!("EmbeddingExtractor::new failed: {e:?}"))?;
    println!("embedding_size = {}", extractor.embedding_size);

    // ---- 4. Compute the embedding for the synthetic sine. ---------------
    println!("extracting embedding");
    let embedding = extractor
        .compute_speaker_embedding(samples, sample_rate)
        .map_err(|e| format!("compute_speaker_embedding failed: {e:?}"))?;

    let len = embedding.len();
    let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
    println!("embedding len = {len}");
    println!("embedding L2 norm = {norm:.4}");

    if len == 0 {
        return Err("embedding length is 0".into());
    }
    if norm == 0.0 || !norm.is_finite() {
        return Err(format!("embedding L2 norm is degenerate: {norm}").into());
    }

    println!("spike OK");
    Ok(())
}
