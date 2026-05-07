use std::path::Path;

use rubato::{Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};

/// Load an audio file (WAV or MP3), downmix to mono f32, return (samples, sample_rate).
pub fn load_wav_mono(path: &Path) -> Result<(Vec<f32>, u32), String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let src = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(src), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| e.to_string())?;

    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or("no audio track")?
        .clone();

    let sample_rate = track.codec_params.sample_rate.unwrap_or(44100);
    let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(1);
    let track_id = track.id;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| e.to_string())?;

    let mut mono_samples: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break
            }
            Err(symphonia::core::errors::Error::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
                buf.copy_interleaved_ref(decoded);
                for chunk in buf.samples().chunks(channels) {
                    mono_samples.push(chunk.iter().sum::<f32>() / channels as f32);
                }
            }
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(e) => return Err(e.to_string()),
        }
    }

    if mono_samples.is_empty() {
        return Err("decoded 0 samples".into());
    }

    Ok((mono_samples, sample_rate))
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
