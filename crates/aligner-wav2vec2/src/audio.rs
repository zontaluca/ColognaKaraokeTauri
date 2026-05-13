//! Audio preprocessing used before wav2vec2 inference.
//!
//! - `highpass_biquad`: removes residual sub-80 Hz energy left by Demucs
//!   vocal separation (kick / room rumble).
//! - `rms_normalize`: scales the buffer so its RMS sits at a target dBFS,
//!   matching the loudness wav2vec2 was trained on.

/// Apply a second-order Butterworth high-pass biquad in-place.
///
/// Coefficients derived for `cutoff_hz` against `sample_rate`. Q = sqrt(2)/2.
pub fn highpass_biquad(samples: &mut [f32], sample_rate: u32, cutoff_hz: f32) {
    if samples.is_empty() || sample_rate == 0 || cutoff_hz <= 0.0 {
        return;
    }
    let sr = sample_rate as f32;
    let q = std::f32::consts::FRAC_1_SQRT_2;
    let w0 = 2.0 * std::f32::consts::PI * cutoff_hz / sr;
    let cos_w0 = w0.cos();
    let sin_w0 = w0.sin();
    let alpha = sin_w0 / (2.0 * q);

    let b0 = (1.0 + cos_w0) / 2.0;
    let b1 = -(1.0 + cos_w0);
    let b2 = (1.0 + cos_w0) / 2.0;
    let a0 = 1.0 + alpha;
    let a1 = -2.0 * cos_w0;
    let a2 = 1.0 - alpha;

    let b0 = b0 / a0;
    let b1 = b1 / a0;
    let b2 = b2 / a0;
    let a1 = a1 / a0;
    let a2 = a2 / a0;

    let mut x1 = 0.0_f32;
    let mut x2 = 0.0_f32;
    let mut y1 = 0.0_f32;
    let mut y2 = 0.0_f32;
    for s in samples.iter_mut() {
        let x0 = *s;
        let y0 = b0 * x0 + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
        x2 = x1;
        x1 = x0;
        y2 = y1;
        y1 = y0;
        *s = y0;
    }
}

/// Scale `samples` so the RMS reaches `target_dbfs`. Clamps the peak to ±0.99
/// after gain so the signal doesn't clip after normalisation.
pub fn rms_normalize(samples: &mut [f32], target_dbfs: f32) {
    if samples.is_empty() {
        return;
    }
    let sum_sq: f64 = samples.iter().map(|s| (*s as f64) * (*s as f64)).sum();
    let rms = (sum_sq / samples.len() as f64).sqrt();
    if rms <= 1e-9 {
        return;
    }
    let target_linear = 10.0_f64.powf(target_dbfs as f64 / 20.0);
    let gain = (target_linear / rms) as f32;

    for s in samples.iter_mut() {
        *s *= gain;
    }

    let peak = samples.iter().map(|s| s.abs()).fold(0.0_f32, f32::max);
    if peak > 0.99 {
        let clip = 0.99 / peak;
        for s in samples.iter_mut() {
            *s *= clip;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highpass_attenuates_dc_offset() {
        let mut buf = vec![1.0_f32; 1600];
        highpass_biquad(&mut buf, 16_000, 80.0);
        let tail_avg: f32 = buf[800..].iter().sum::<f32>() / 800.0;
        assert!(tail_avg.abs() < 0.05, "DC not removed: tail avg = {}", tail_avg);
    }

    #[test]
    fn rms_normalize_hits_target_dbfs() {
        let mut buf: Vec<f32> = (0..16_000)
            .map(|i| 0.01 * (i as f32 * 0.05).sin())
            .collect();
        rms_normalize(&mut buf, -20.0);

        let sum_sq: f64 = buf.iter().map(|s| (*s as f64) * (*s as f64)).sum();
        let rms = (sum_sq / buf.len() as f64).sqrt();
        let dbfs = 20.0 * rms.log10();
        assert!((dbfs - -20.0).abs() < 0.5, "got {} dBFS", dbfs);
    }
}
