use std::path::{Path, PathBuf};

use regex::Regex;
use tauri::AppHandle;
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;

pub async fn separate_vocals(
    app: &AppHandle,
    input_path: &Path,
    output_dir: &Path,
    mut on_progress: impl FnMut(&str, f32),
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(output_dir).map_err(|e| e.to_string())?;
    on_progress("Starting vocal separation...", 0.0);

    let stems_dir = output_dir.join("_stems_tmp");
    std::fs::create_dir_all(&stems_dir).map_err(|e| e.to_string())?;

    let input_str = input_path.to_string_lossy().to_string();
    let out_str = stems_dir.to_string_lossy().to_string();

    let (mut rx, _child) = app
        .shell()
        .sidecar("demucs")
        .map_err(|e| e.to_string())?
        .args([
            &input_str,
            "-m",
            "htdemucs",
            "-s",
            "drums,bass,other,vocals",
            "-o",
            &out_str,
        ])
        .spawn()
        .map_err(|e| e.to_string())?;

    let pct_re = Regex::new(r"(\d{1,3})%").unwrap();

    while let Some(event) = rx.recv().await {
        match event {
            CommandEvent::Stdout(bytes) | CommandEvent::Stderr(bytes) => {
                let line = String::from_utf8_lossy(&bytes).to_string();
                if let Some(m) = pct_re.captures_iter(&line).last() {
                    if let Ok(p) = m[1].parse::<f32>() {
                        on_progress(
                            &format!("Separating... {:.0}%", p),
                            (p / 100.0).clamp(0.0, 1.0),
                        );
                    }
                }
            }
            CommandEvent::Error(err) => return Err(err),
            CommandEvent::Terminated(payload) => {
                if payload.code.unwrap_or(-1) != 0 {
                    return Err(format!("demucs exited with code {:?}", payload.code));
                }
                break;
            }
            _ => {}
        }
    }

    // demucs writes stems: drums.wav, bass.wav, other.wav, vocals.wav
    let drums = stems_dir.join("drums.wav");
    let bass = stems_dir.join("bass.wav");
    let other = stems_dir.join("other.wav");

    if !drums.exists() || !bass.exists() || !other.exists() {
        return Err(format!(
            "Expected stems not found in {}",
            stems_dir.display()
        ));
    }

    on_progress("Encoding audio...", 0.95);

    let instrumental_path = output_dir.join("instrumental.mp3");
    let vocals_stem = stems_dir.join("vocals.wav");
    let vocals_out = output_dir.join("vocals.mp3");
    let instrumental_out = instrumental_path.clone();
    // Mixing and MP3 encoding are CPU-bound: run them off the async runtime,
    // encoding the instrumental and the vocals in parallel on two threads.
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        std::thread::scope(|scope| {
            let vocals_job = scope.spawn(|| {
                if vocals_stem.exists() {
                    encode_vocals_mp3(&vocals_stem, &vocals_out)
                        .map_err(|e| format!("Vocals encode failed: {}", e))
                } else {
                    Ok(())
                }
            });

            let instrumental = mix_wavs_to_memory(&[&drums, &bass, &other])
                .map_err(|e| format!("Mix failed: {}", e))
                .and_then(|(mixed, spec)| {
                    encode_stereo_mp3(&mixed, spec.channels, spec.sample_rate, &instrumental_out)
                        .map_err(|e| format!("Instrumental encode failed: {}", e))
                });
            let vocals = vocals_job
                .join()
                .unwrap_or_else(|_| Err("Vocals encode thread panicked".into()));
            instrumental?;
            vocals
        })
    })
    .await
    .map_err(|e| e.to_string())??;

    let _ = std::fs::remove_dir_all(&stems_dir);

    on_progress("Separation complete", 1.0);
    Ok(instrumental_path)
}

/// PCM frames handed to LAME per call: bounds the i16 scratch buffers instead
/// of converting a whole song up front.
const ENCODE_CHUNK_FRAMES: usize = 1152 * 64;

fn to_i16(s: f32) -> i16 {
    (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}

fn open_wav(path: &Path) -> Result<hound::WavReader<std::io::BufReader<std::fs::File>>, String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    hound::WavReader::new(std::io::BufReader::new(f)).map_err(|e| e.to_string())
}

/// Iterate a WAV's samples (all channels, interleaved) as f32 in [-1, 1].
/// Unreadable samples decode as silence, as before.
fn wav_samples_f32<'a, R: std::io::Read + 'a>(
    reader: &'a mut hound::WavReader<R>,
) -> Box<dyn Iterator<Item = f32> + 'a> {
    let spec = reader.spec();
    if spec.sample_format == hound::SampleFormat::Float {
        Box::new(reader.samples::<f32>().map(|s| s.unwrap_or(0.0)))
    } else {
        let bits = spec.bits_per_sample as i32;
        let max = (1i64 << (bits - 1)) as f32;
        Box::new(reader.samples::<i32>().map(move |s| s.unwrap_or(0) as f32 / max))
    }
}

/// Mix PCM WAV files of identical format in memory; normalize to prevent clipping.
/// Each stem is streamed into a single accumulator, so only the mix is held in
/// memory (not every stem at once).
fn mix_wavs_to_memory(inputs: &[&Path]) -> Result<(Vec<f32>, hound::WavSpec), String> {
    if inputs.is_empty() {
        return Err("no inputs".into());
    }

    let mut readers: Vec<_> = inputs
        .iter()
        .map(|p| open_wav(p))
        .collect::<Result<Vec<_>, _>>()?;

    let spec = readers[0].spec();
    for r in &readers[1..] {
        if r.spec() != spec {
            return Err("WAV specs differ".into());
        }
    }

    let len = readers.iter().map(|r| r.len() as usize).min().unwrap_or(0);
    let mut mixed: Vec<f32> = vec![0.0; len];
    for reader in readers.iter_mut() {
        for (dst, s) in mixed.iter_mut().zip(wav_samples_f32(reader)) {
            *dst += s;
        }
    }

    let peak = mixed.iter().cloned().map(f32::abs).fold(0.0_f32, f32::max);
    if peak > 1.0 {
        for s in mixed.iter_mut() {
            *s /= peak;
        }
    }

    Ok((mixed, spec))
}

fn build_encoder(
    channels: u8,
    sample_rate: u32,
    bitrate: mp3lame_encoder::Bitrate,
) -> Result<mp3lame_encoder::Encoder, String> {
    let mut b = mp3lame_encoder::Builder::new().ok_or("failed to create mp3 builder")?;
    b.set_num_channels(channels)
        .map_err(|e| format!("{e:?}"))?;
    b.set_sample_rate(sample_rate)
        .map_err(|e| format!("{e:?}"))?;
    b.set_brate(bitrate)
        .map_err(|e| format!("{e:?}"))?;
    b.build().map_err(|e| format!("{e:?}"))
}

fn encode_stereo_mp3(
    samples: &[f32],
    channels: u16,
    sample_rate: u32,
    out: &Path,
) -> Result<(), String> {
    use mp3lame_encoder::{Bitrate, DualPcm, FlushNoGap, MonoPcm, max_required_buffer_size};

    let mut enc = build_encoder(channels as u8, sample_rate, Bitrate::Kbps192)?;

    let ch = channels.max(1) as usize;
    let n_samples = samples.len() / ch;
    let mut mp3 = Vec::with_capacity(max_required_buffer_size(n_samples) + 7200);

    if ch == 1 {
        let mut pcm: Vec<i16> = Vec::with_capacity(ENCODE_CHUNK_FRAMES);
        for chunk in samples.chunks(ENCODE_CHUNK_FRAMES) {
            pcm.clear();
            pcm.extend(chunk.iter().map(|&s| to_i16(s)));
            mp3.reserve(max_required_buffer_size(pcm.len()));
            enc.encode_to_vec(MonoPcm(&pcm), &mut mp3)
                .map_err(|e| format!("{e:?}"))?;
        }
    } else {
        let mut left: Vec<i16> = Vec::with_capacity(ENCODE_CHUNK_FRAMES);
        let mut right: Vec<i16> = Vec::with_capacity(ENCODE_CHUNK_FRAMES);
        for chunk in samples.chunks(ENCODE_CHUNK_FRAMES * ch) {
            left.clear();
            right.clear();
            for frame in chunk.chunks_exact(ch) {
                left.push(to_i16(frame[0]));
                right.push(to_i16(frame[1]));
            }
            mp3.reserve(max_required_buffer_size(left.len()));
            enc.encode_to_vec(DualPcm { left: &left, right: &right }, &mut mp3)
                .map_err(|e| format!("{e:?}"))?;
        }
    }
    mp3.reserve(7200);
    enc.flush_to_vec::<FlushNoGap>(&mut mp3)
        .map_err(|e| format!("{e:?}"))?;
    std::fs::write(out, mp3).map_err(|e| e.to_string())?;
    Ok(())
}

/// Read a WAV stem, downmix to mono, encode as 320 kbps MP3.
/// Streams the stem through a bounded buffer instead of loading it whole.
fn encode_vocals_mp3(wav_path: &Path, out: &Path) -> Result<(), String> {
    use mp3lame_encoder::{Bitrate, FlushNoGap, MonoPcm, max_required_buffer_size};

    let mut reader = open_wav(wav_path)?;
    let spec = reader.spec();
    let channels = (spec.channels as usize).max(1);
    let total_frames = reader.duration() as usize;

    let mut enc = build_encoder(1, spec.sample_rate, Bitrate::Kbps320)?;
    let mut mp3 = Vec::with_capacity(max_required_buffer_size(total_frames) + 7200);

    let mut mono: Vec<i16> = Vec::with_capacity(ENCODE_CHUNK_FRAMES);
    let inv = 1.0 / channels as f32;
    let mut acc = 0.0_f32;
    let mut in_frame = 0usize;
    for s in wav_samples_f32(&mut reader) {
        acc += s;
        in_frame += 1;
        if in_frame < channels {
            continue;
        }
        mono.push(to_i16(acc * inv));
        acc = 0.0;
        in_frame = 0;
        if mono.len() == ENCODE_CHUNK_FRAMES {
            mp3.reserve(max_required_buffer_size(mono.len()));
            enc.encode_to_vec(MonoPcm(&mono), &mut mp3)
                .map_err(|e| format!("{e:?}"))?;
            mono.clear();
        }
    }
    if !mono.is_empty() {
        mp3.reserve(max_required_buffer_size(mono.len()));
        enc.encode_to_vec(MonoPcm(&mono), &mut mp3)
            .map_err(|e| format!("{e:?}"))?;
    }
    mp3.reserve(7200);
    enc.flush_to_vec::<FlushNoGap>(&mut mp3)
        .map_err(|e| format!("{e:?}"))?;
    std::fs::write(out, mp3).map_err(|e| e.to_string())?;
    Ok(())
}
