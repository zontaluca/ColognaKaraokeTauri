use std::path::Path;

use rubato::{Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};

/// Load a WAV file, downmix to mono f32 at given target sample rate.
pub fn load_wav_mono(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let reader = hound::WavReader::open(path).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    let channels = spec.channels as usize;
    let sample_rate = spec.sample_rate;

    let samples_f32: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .map(|s| s.unwrap_or(0.0))
            .collect(),
        hound::SampleFormat::Int => {
            let bits = spec.bits_per_sample as i32;
            let max = (1i64 << (bits - 1)) as f32;
            reader
                .into_samples::<i32>()
                .map(|s| s.unwrap_or(0) as f32 / max)
                .collect()
        }
    };

    let mono: Vec<f32> = if channels <= 1 {
        samples_f32
    } else {
        samples_f32
            .chunks(channels)
            .map(|c| c.iter().sum::<f32>() / channels as f32)
            .collect()
    };
    Ok((mono, sample_rate))
}

pub fn resample_to(input: &[f32], src_rate: u32, dst_rate: u32) -> Result<Vec<f32>, String> {
    if src_rate == dst_rate {
        return Ok(input.to_vec());
    }
    let params = SincInterpolationParameters {
        sinc_len: 128,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 128,
        window: WindowFunction::BlackmanHarris2,
    };
    let ratio = dst_rate as f64 / src_rate as f64;
    let mut resampler = SincFixedIn::<f32>::new(ratio, 2.0, params, input.len(), 1)
        .map_err(|e| e.to_string())?;
    let waves_in = vec![input.to_vec()];
    let waves_out = resampler.process(&waves_in, None).map_err(|e| e.to_string())?;
    Ok(waves_out.into_iter().next().unwrap_or_default())
}

pub fn load_wav_mono_16k(path: &Path) -> Result<Vec<f32>, String> {
    let (mono, sr) = load_wav_mono(path)?;
    resample_to(&mono, sr, 16000)
}
