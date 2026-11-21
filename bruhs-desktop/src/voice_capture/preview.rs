//! Bounded, non-overlapping local preview chunks. The final pass uses the full recording.
use super::*;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Instant;
use tokio::sync::{mpsc, watch};

pub(super) fn record(
    buffer: &Mutex<Vec<f32>>,
    rate: u32,
    stop: Receiver<bool>,
    limit: Duration,
    chunks: mpsc::Sender<Vec<u8>>,
) -> bool {
    let start = Instant::now();
    let mut offset = 0usize;
    loop {
        let remaining = limit.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            return true;
        }
        match stop.recv_timeout(remaining.min(Duration::from_millis(200))) {
            Ok(save) => return save,
            Err(RecvTimeoutError::Disconnected) => return false,
            Err(RecvTimeoutError::Timeout) => {}
        }
        // Schema permits a single queued chunk and a single decoder. Audio stays
        // in the original buffer until capacity returns, including on slower hosts.
        if let Ok(slot) = chunks.try_reserve() {
            let frames = buffer.lock().ok().and_then(|pcm| {
                let pending = pcm.get(offset..)?;
                let count = chunk_end(pending, rate as usize)?;
                Some(pending[..count].to_vec())
            });
            if let Some(frames) = frames {
                if let Ok(wav) = encode_wav_16bit_mono(&frames, rate) {
                    offset += frames.len();
                    slot.send(wav);
                }
            }
        }
    }
}

fn chunk_end(pcm: &[f32], rate: usize) -> Option<usize> {
    if rate == 0 || pcm.len() < rate * 2 {
        return None;
    }
    let max = (rate * 8).min(pcm.len());
    // Prefer the latest quiet 200 ms after two seconds; force a boundary for
    // continuous speech. Only the final transcription is inserted into the draft.
    let width = (rate / 5).max(1);
    if let Some((i, _)) = pcm[..max]
        .chunks_exact(width)
        .enumerate()
        .rev()
        .take_while(|(i, _)| (i + 1) * width >= rate * 2)
        .find(|(_, chunk)| chunk.iter().all(|s| s.abs() < 0.005))
    {
        return Some((i + 1) * width);
    }
    (max == rate * 8).then_some(max)
}

pub async fn transcribe(
    mut chunks: mpsc::Receiver<Vec<u8>>,
    options: bruhs_models::local_tools::DictationOptions,
    updates: watch::Sender<String>,
) {
    let mut preview = String::new();
    while let Some(wav) = chunks.recv().await {
        if chunks.is_closed() || updates.is_closed() {
            break;
        }
        let Ok(reply) = bruhs_core::capture_client::transcribe_dictation(
            bruhs_core::capture_client::endpoint(),
            &options,
            wav,
        )
        .await
        else {
            continue;
        };
        let words = reply.text.trim();
        if !words.is_empty() {
            if !preview.is_empty() {
                preview.push(' ');
            }
            preview.push_str(words);
            updates.send_replace(preview.clone());
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires a private capture host and a WAV fixture with speech"]
    async fn local_preview_does_not_duplicate_the_final_draft() {
        let bytes =
            std::fs::read(std::env::var_os("BRUHS_TEST_DICTATION_WAV").expect("WAV fixture"))
                .unwrap();
        let options = bruhs_models::local_tools::DictationOptions {
            model: bruhs_models::WhisperModel::TinyEn,
            language: Some("en".into()),
            silence_filtering: false,
            remove_filler_words: false,
        };
        let (chunks, audio) = mpsc::channel(1);
        let (updates, mut preview) = watch::channel(String::new());
        let decoder = tokio::spawn(transcribe(audio, options.clone(), updates));
        chunks.send(bytes.clone()).await.unwrap();
        preview.changed().await.unwrap();
        let first = preview.borrow_and_update().clone();
        assert!(!first.is_empty());
        chunks.send(bytes.clone()).await.unwrap();
        preview.changed().await.unwrap();
        assert_eq!(*preview.borrow(), format!("{first} {first}"));
        drop(chunks);
        decoder.await.unwrap();
        let final_text = bruhs_core::capture_client::transcribe_dictation(
            bruhs_core::capture_client::endpoint(),
            &options,
            bytes,
        )
        .await
        .unwrap()
        .text;
        let draft =
            bruhs_models::local_tools::insert_dictation("Draft", "Draft", 5..5, &final_text)
                .unwrap();
        assert!(draft.ends_with(&first));
        assert_eq!(draft.matches(&first).count(), 1);
    }

    #[test]
    fn preview_chunks_prefer_a_pause_and_bound_continuous_speech() {
        assert_eq!(chunk_end(&vec![1.; 199], 100), None);
        let mut speech = vec![0.5; 350];
        speech[260..280].fill(0.);
        assert_eq!(chunk_end(&speech, 100), Some(280));
        assert_eq!(chunk_end(&vec![1.; 950], 100), Some(800));
    }
}
