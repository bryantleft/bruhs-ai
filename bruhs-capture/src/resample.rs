//! Shared mono FFT resampling for the capture daemon.

use audioadapter_buffers::direct::InterleavedSlice;
use bruhs_models::logic::wav::resampled_len;
use rubato::{
    Fft, FixedSync, Indexing, ResampleError as ProcessError, Resampler, ResamplerConstructionError,
    WindowFunction,
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum ResampleError {
    #[error(transparent)]
    Construction(#[from] ResamplerConstructionError),
    #[error(transparent)]
    Buffer(#[from] audioadapter_buffers::SizeError),
    #[error(transparent)]
    Process(#[from] ProcessError),
}

pub(crate) struct MonoResampler {
    fft: Fft<f32>,
    output: Vec<f32>,
}

impl MonoResampler {
    pub(crate) fn new(frames: usize, from_rate: u32, to_rate: u32) -> Result<Self, ResampleError> {
        let chunk_size = (from_rate as usize / 4).max(256).min(frames.max(256));
        // Match the previous FftFixedIn filter and sub-chunk size so the
        // migration preserves the audio fed to Whisper and the speaker model.
        let fft = Fft::new_custom(
            from_rate as usize,
            to_rate as usize,
            chunk_size,
            2,
            1,
            WindowFunction::BlackmanHarris2,
            FixedSync::Input,
        )?;
        let output = vec![0.0; fft.output_frames_max()];
        Ok(Self { fft, output })
    }

    pub(crate) fn chunk_size(&self) -> usize {
        self.fft.input_frames_next()
    }

    /// Process one chunk, padding a short final chunk with silence.
    pub(crate) fn process(&mut self, input: &[f32]) -> Result<&[f32], ResampleError> {
        let source = InterleavedSlice::new(input, 1, input.len())?;
        let capacity = self.output.len();
        let mut target = InterleavedSlice::new_mut(&mut self.output, 1, capacity)?;
        let indexing = Indexing {
            partial_len: Some(input.len()),
            ..Indexing::default()
        };
        let (_, produced) = self
            .fft
            .process_into_buffer(&source, &mut target, Some(&indexing))?;
        Ok(&self.output[..produced])
    }
}

pub(crate) fn resample_mono(
    input: &[f32],
    from_rate: u32,
    to_rate: u32,
) -> Result<Vec<f32>, ResampleError> {
    let mut resampler = MonoResampler::new(input.len(), from_rate, to_rate)?;
    let exact_out = resampled_len(input.len(), from_rate, to_rate);
    let mut output = Vec::with_capacity(exact_out);
    for chunk in input.chunks(resampler.chunk_size()) {
        let remaining = exact_out - output.len();
        let samples = resampler.process(chunk)?;
        output.extend_from_slice(&samples[..samples.len().min(remaining)]);
    }
    Ok(output)
}
