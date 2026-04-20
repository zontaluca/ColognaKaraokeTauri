//! Log-mel spectrogram matching Whisper's preprocessing.
//!
//! Parameters (fixed by the Whisper architecture):
//!   sample_rate = 16 000 Hz
//!   n_fft       = 400 (25 ms window)
//!   hop_length  = 160 (10 ms hop)
//!   n_mels      = 80  (small / medium) or 128 (large-v3)
//!   fmin        = 0 Hz, fmax = 8 000 Hz

use rustfft::{num_complex::Complex, FftPlanner};

pub const SAMPLE_RATE: u32 = 16_000;
pub const N_FFT: usize = 400;
pub const HOP_LENGTH: usize = 160;
/// One encoder frame = 2 mel frames = 20 ms.
pub const FRAME_MS: f64 = 20.0;
/// Encoder frame count for a 30-second chunk (1 500 frames).
pub const ENCODER_FRAMES: usize = 1_500;

/// Compute the log-mel spectrogram from raw 16 kHz mono PCM.
///
/// Returns a 2-D matrix `[n_mels × n_frames]`.  `n_frames` is
/// `ceil((samples.len() + N_FFT/2) / HOP_LENGTH)`.
pub fn log_mel_spectrogram(samples: &[f32], n_mels: usize) -> Vec<Vec<f32>> {
    let padded = pad_reflect(samples, N_FFT / 2);
    let n_frames = 1 + (padded.len() - N_FFT) / HOP_LENGTH;

    let window = hann_window(N_FFT);
    let mel_fb = mel_filterbank(n_mels, N_FFT, SAMPLE_RATE, 0.0, 8000.0);

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(N_FFT);

    let mut mel_spec: Vec<Vec<f32>> = vec![vec![0.0; n_frames]; n_mels];

    for frame_idx in 0..n_frames {
        let start = frame_idx * HOP_LENGTH;
        let mut buf: Vec<Complex<f32>> = padded[start..start + N_FFT]
            .iter()
            .zip(window.iter())
            .map(|(&s, &w)| Complex::new(s * w, 0.0))
            .collect();

        fft.process(&mut buf);

        // Power spectrum (only positive frequencies: 0..=N_FFT/2).
        let n_freqs = N_FFT / 2 + 1;
        let power: Vec<f32> = (0..n_freqs)
            .map(|k| buf[k].norm_sqr())
            .collect();

        // Apply mel filterbank.
        for (m, filter) in mel_fb.iter().enumerate() {
            let energy: f32 = filter.iter().zip(power.iter()).map(|(f, p)| f * p).sum();
            mel_spec[m][frame_idx] = energy;
        }
    }

    // Log and normalize (Whisper uses max-10 normalization).
    for row in &mut mel_spec {
        for v in row.iter_mut() {
            *v = (v.max(1e-10)).log10();
        }
    }
    let max_val = mel_spec
        .iter()
        .flat_map(|row| row.iter())
        .cloned()
        .fold(f32::NEG_INFINITY, f32::max);
    for row in &mut mel_spec {
        for v in row.iter_mut() {
            *v = ((*v).max(max_val - 8.0) + 4.0) / 4.0;
        }
    }

    mel_spec
}

/// Pad signal with reflect padding of `pad` samples on each side.
fn pad_reflect(samples: &[f32], pad: usize) -> Vec<f32> {
    let n = samples.len();
    let mut out = Vec::with_capacity(n + 2 * pad);
    // Left pad: mirror samples[1..pad+1] reversed.
    for i in (1..=pad).rev() {
        out.push(samples[i.min(n - 1)]);
    }
    out.extend_from_slice(samples);
    // Right pad: mirror samples[n-pad-1..n-1] reversed.
    for i in 1..=pad {
        out.push(samples[(n - 1 - i).max(0)]);
    }
    out
}

/// Hann window of length `n`.
fn hann_window(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (n - 1) as f32).cos()))
        .collect()
}

/// Build a triangular mel filterbank.
/// Returns `n_mels` vectors each of length `n_fft/2 + 1`.
fn mel_filterbank(
    n_mels: usize,
    n_fft: usize,
    sample_rate: u32,
    fmin: f32,
    fmax: f32,
) -> Vec<Vec<f32>> {
    let n_freqs = n_fft / 2 + 1;
    let mel_min = hz_to_mel(fmin);
    let mel_max = hz_to_mel(fmax);

    // n_mels + 2 evenly spaced mel points.
    let mel_points: Vec<f32> = (0..=n_mels + 1)
        .map(|i| mel_min + (mel_max - mel_min) * i as f32 / (n_mels + 1) as f32)
        .collect();
    let hz_points: Vec<f32> = mel_points.iter().map(|&m| mel_to_hz(m)).collect();

    // Map to FFT bin indices.
    let bin_points: Vec<usize> = hz_points
        .iter()
        .map(|&f| ((n_fft + 1) as f32 * f / sample_rate as f32).floor() as usize)
        .collect();

    let mut filters = vec![vec![0.0f32; n_freqs]; n_mels];
    for m in 0..n_mels {
        let left = bin_points[m];
        let center = bin_points[m + 1];
        let right = bin_points[m + 2];

        for k in left..center {
            if k < n_freqs && center > left {
                filters[m][k] = (k - left) as f32 / (center - left) as f32;
            }
        }
        for k in center..right {
            if k < n_freqs && right > center {
                filters[m][k] = (right - k) as f32 / (right - center) as f32;
            }
        }
    }
    filters
}

fn hz_to_mel(hz: f32) -> f32 {
    2595.0 * (1.0 + hz / 700.0).log10()
}

fn mel_to_hz(mel: f32) -> f32 {
    700.0 * (10f32.powf(mel / 2595.0) - 1.0)
}

/// Split a samples buffer into overlapping 30-second chunks.
/// Returns (chunk_samples, chunk_offset_seconds).
pub fn make_chunks(
    samples: &[f32],
    chunk_seconds: f32,
    overlap_seconds: f32,
) -> Vec<(Vec<f32>, f64)> {
    let sr = SAMPLE_RATE as usize;
    let chunk_len = (chunk_seconds * sr as f32) as usize;
    let hop_len = ((chunk_seconds - overlap_seconds) * sr as f32) as usize;

    if samples.len() <= chunk_len {
        return vec![(samples.to_vec(), 0.0)];
    }

    let mut chunks = Vec::new();
    let mut start = 0;
    while start < samples.len() {
        let end = (start + chunk_len).min(samples.len());
        let mut chunk = samples[start..end].to_vec();
        // Zero-pad last chunk to full length so mel always has ENCODER_FRAMES cols.
        if chunk.len() < chunk_len {
            chunk.resize(chunk_len, 0.0);
        }
        let offset = start as f64 / sr as f64;
        chunks.push((chunk, offset));
        if end == samples.len() {
            break;
        }
        start += hop_len;
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mel_shape() {
        let samples = vec![0.0f32; SAMPLE_RATE as usize]; // 1 second of silence
        let mel = log_mel_spectrogram(&samples, 80);
        assert_eq!(mel.len(), 80);
        assert!(!mel[0].is_empty());
    }

    #[test]
    fn chunks_no_overlap_short() {
        let samples = vec![0.0f32; SAMPLE_RATE as usize * 10]; // 10s
        let chunks = make_chunks(&samples, 30.0, 3.0);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].1, 0.0);
    }

    #[test]
    fn chunks_multiple() {
        let samples = vec![0.0f32; SAMPLE_RATE as usize * 90]; // 90s
        let chunks = make_chunks(&samples, 30.0, 3.0);
        assert!(chunks.len() >= 3);
        // Offsets must be strictly increasing.
        for w in chunks.windows(2) {
            assert!(w[1].1 > w[0].1);
        }
    }
}
