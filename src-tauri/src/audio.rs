use std::path::Path;

use rubato::{Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};

/// Load an audio file (WAV or MP3), downmix to mono f32, return (samples, sample_rate).
pub fn load_wav_mono(path: &Path) -> Result<(Vec<f32>, u32), String> {
    decode_mono(path, None)
}

/// Decode only `seconds` of audio centred on the middle of the track. When the
/// container reports its length, decoding stops right after the excerpt and
/// nothing before it is kept; otherwise the whole file is decoded and sliced.
pub fn load_mono_middle(path: &Path, seconds: f32) -> Result<(Vec<f32>, u32), String> {
    let (samples, sample_rate) = decode_mono(path, Some(seconds))?;
    let want = ((seconds * sample_rate as f32) as usize).min(samples.len());
    if samples.len() <= want {
        return Ok((samples, sample_rate));
    }
    // Container length unknown: the full track was decoded, keep the middle.
    let start = samples.len() / 2 - want / 2;
    Ok((samples[start..start + want].to_vec(), sample_rate))
}

fn decode_mono(path: &Path, middle_secs: Option<f32>) -> Result<(Vec<f32>, u32), String> {
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
    let track_id = track.id;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| e.to_string())?;

    // Pre-size the output when the container reports its length, so a full song
    // doesn't go through a dozen reallocations while growing.
    let expected_frames = track.codec_params.n_frames.unwrap_or(0) as usize;
    // Frame range [start, end) to keep, when an excerpt was requested and the
    // total length is known up front.
    let window = match middle_secs {
        Some(secs) if expected_frames > 0 => {
            let want = ((secs * sample_rate as f32) as usize).min(expected_frames);
            let start = expected_frames / 2 - want / 2;
            Some((start, start + want))
        }
        _ => None,
    };
    let capacity = window.map_or(expected_frames, |(start, end)| end - start);
    let mut mono_samples: Vec<f32> = Vec::with_capacity(capacity);
    let mut frame_pos = 0usize;
    // Interleaved scratch buffer reused across packets; reallocated only when a
    // packet needs more room than the previous ones.
    let mut sample_buf: Option<SampleBuffer<f32>> = None;
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
                let channels = spec.channels.count().max(1);
                let needed = decoded.capacity() * channels;
                if sample_buf.as_ref().map_or(true, |b| b.capacity() < needed) {
                    sample_buf = Some(SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
                }
                let buf = sample_buf.as_mut().expect("sample buffer allocated above");
                buf.copy_interleaved_ref(decoded);
                let interleaved = buf.samples();
                let packet_start = frame_pos;
                frame_pos += interleaved.len() / channels;
                match window {
                    None => downmix_into(interleaved, channels, &mut mono_samples),
                    Some((start, end)) => {
                        let lo = start.max(packet_start);
                        let hi = end.min(frame_pos);
                        if lo < hi {
                            let range = (lo - packet_start) * channels..(hi - packet_start) * channels;
                            downmix_into(&interleaved[range], channels, &mut mono_samples);
                        }
                        if frame_pos >= end {
                            break;
                        }
                    }
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

/// Append the mono average of an interleaved buffer to `out`.
fn downmix_into(interleaved: &[f32], channels: usize, out: &mut Vec<f32>) {
    if channels <= 1 {
        out.extend_from_slice(interleaved);
        return;
    }
    let inv = 1.0 / channels as f32;
    out.extend(
        interleaved
            .chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() * inv),
    );
}

pub fn resample_to(input: &[f32], src_rate: u32, dst_rate: u32) -> Result<Vec<f32>, String> {
    if src_rate == dst_rate {
        return Ok(input.to_vec());
    }
    if input.is_empty() {
        return Ok(Vec::new());
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
    // Borrow the input directly instead of copying the whole signal first.
    let waves_out = resampler.process(&[input], None).map_err(|e| e.to_string())?;
    Ok(waves_out.into_iter().next().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_averages_channels() {
        let mut out = Vec::new();
        downmix_into(&[1.0, 0.0, 0.5, 0.5], 2, &mut out);
        assert_eq!(out, vec![0.5, 0.5]);
    }

    #[test]
    fn resample_halves_length() {
        let input: Vec<f32> = (0..32_000).map(|i| (i as f32 * 0.01).sin()).collect();
        let out = resample_to(&input, 32_000, 16_000).unwrap();
        assert!((out.len() as i64 - 16_000).abs() < 200, "len = {}", out.len());
    }
}
