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

    let (mixed, spec) = mix_wavs_to_memory(&[&drums, &bass, &other])
        .map_err(|e| format!("Mix failed: {}", e))?;
    let instrumental_path = output_dir.join("instrumental.mp3");
    encode_stereo_mp3(&mixed, spec.channels, spec.sample_rate, &instrumental_path)
        .map_err(|e| format!("Instrumental encode failed: {}", e))?;

    let vocals_stem = stems_dir.join("vocals.wav");
    if vocals_stem.exists() {
        encode_vocals_mp3(&vocals_stem, &output_dir.join("vocals.mp3"))
            .map_err(|e| format!("Vocals encode failed: {}", e))?;
    }

    let _ = std::fs::remove_dir_all(&stems_dir);

    on_progress("Separation complete", 1.0);
    Ok(instrumental_path)
}

/// Mix PCM WAV files of identical format in memory; normalize to prevent clipping.
fn mix_wavs_to_memory(inputs: &[&Path]) -> Result<(Vec<f32>, hound::WavSpec), String> {
    use std::io::BufReader;

    if inputs.is_empty() {
        return Err("no inputs".into());
    }

    let mut readers: Vec<_> = inputs
        .iter()
        .map(|p| {
            let f = std::fs::File::open(p).map_err(|e| e.to_string())?;
            hound::WavReader::new(BufReader::new(f)).map_err(|e| e.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;

    let spec = readers[0].spec();
    for r in &readers[1..] {
        if r.spec() != spec {
            return Err("WAV specs differ".into());
        }
    }

    let all_samples: Vec<Vec<f32>> = readers
        .iter_mut()
        .map(|r| {
            if spec.sample_format == hound::SampleFormat::Float {
                r.samples::<f32>()
                    .map(|s| s.unwrap_or(0.0))
                    .collect::<Vec<_>>()
            } else {
                let bits = spec.bits_per_sample as i32;
                let max = (1i64 << (bits - 1)) as f32;
                r.samples::<i32>()
                    .map(|s| s.unwrap_or(0) as f32 / max)
                    .collect::<Vec<_>>()
            }
        })
        .collect();

    let len = all_samples.iter().map(|v| v.len()).min().unwrap_or(0);
    let mut mixed: Vec<f32> = Vec::with_capacity(len);
    for i in 0..len {
        mixed.push(all_samples.iter().map(|v| v[i]).sum());
    }

    let peak = mixed.iter().cloned().map(f32::abs).fold(0.0_f32, f32::max);
    if peak > 1.0 {
        for s in mixed.iter_mut() {
            *s /= peak;
        }
    }

    Ok((mixed, spec))
}

fn encode_stereo_mp3(
    samples: &[f32],
    channels: u16,
    sample_rate: u32,
    out: &Path,
) -> Result<(), String> {
    use mp3lame_encoder::{Bitrate, Builder, DualPcm, FlushNoGap, MonoPcm, max_required_buffer_size};

    let mut b = Builder::new().ok_or("failed to create mp3 builder")?;
    b.set_num_channels(channels as u8)
        .map_err(|e| format!("{e:?}"))?;
    b.set_sample_rate(sample_rate)
        .map_err(|e| format!("{e:?}"))?;
    b.set_brate(Bitrate::Kbps192)
        .map_err(|e| format!("{e:?}"))?;
    let mut enc = b.build().map_err(|e| format!("{e:?}"))?;

    let n_samples = if channels <= 1 { samples.len() } else { samples.len() / 2 };
    let mut mp3 = Vec::with_capacity(max_required_buffer_size(n_samples) + 7200);

    if channels == 1 {
        let pcm: Vec<i16> = samples
            .iter()
            .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .collect();
        enc.encode_to_vec(MonoPcm(&pcm), &mut mp3)
            .map_err(|e| format!("{e:?}"))?;
    } else {
        let left: Vec<i16> = samples
            .iter()
            .step_by(2)
            .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .collect();
        let right: Vec<i16> = samples
            .iter()
            .skip(1)
            .step_by(2)
            .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .collect();
        enc.encode_to_vec(DualPcm { left: &left, right: &right }, &mut mp3)
            .map_err(|e| format!("{e:?}"))?;
    }
    enc.flush_to_vec::<FlushNoGap>(&mut mp3)
        .map_err(|e| format!("{e:?}"))?;
    std::fs::write(out, mp3).map_err(|e| e.to_string())?;
    Ok(())
}

/// Read a WAV stem, downmix to mono, encode as 320 kbps MP3.
fn encode_vocals_mp3(wav_path: &Path, out: &Path) -> Result<(), String> {
    use std::io::BufReader;

    use mp3lame_encoder::{Bitrate, Builder, FlushNoGap, MonoPcm, max_required_buffer_size};

    let f = std::fs::File::open(wav_path).map_err(|e| e.to_string())?;
    let mut reader = hound::WavReader::new(BufReader::new(f)).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    let channels = spec.channels as usize;

    let samples_f32: Vec<f32> = if spec.sample_format == hound::SampleFormat::Float {
        reader
            .samples::<f32>()
            .map(|s| s.unwrap_or(0.0))
            .collect()
    } else {
        let bits = spec.bits_per_sample as i32;
        let max = (1i64 << (bits - 1)) as f32;
        reader
            .samples::<i32>()
            .map(|s| s.unwrap_or(0) as f32 / max)
            .collect()
    };

    let mono: Vec<i16> = if channels <= 1 {
        samples_f32
            .iter()
            .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .collect()
    } else {
        samples_f32
            .chunks(channels)
            .map(|c| {
                let avg = c.iter().sum::<f32>() / channels as f32;
                (avg.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
            })
            .collect()
    };

    let mut b = Builder::new().ok_or("failed to create mp3 builder")?;
    b.set_num_channels(1).map_err(|e| format!("{e:?}"))?;
    b.set_sample_rate(spec.sample_rate)
        .map_err(|e| format!("{e:?}"))?;
    b.set_brate(Bitrate::Kbps320)
        .map_err(|e| format!("{e:?}"))?;
    let mut enc = b.build().map_err(|e| format!("{e:?}"))?;

    let mut mp3 = Vec::with_capacity(max_required_buffer_size(mono.len()) + 7200);
    enc.encode_to_vec(MonoPcm(&mono), &mut mp3)
        .map_err(|e| format!("{e:?}"))?;
    enc.flush_to_vec::<FlushNoGap>(&mut mp3)
        .map_err(|e| format!("{e:?}"))?;
    std::fs::write(out, mp3).map_err(|e| e.to_string())?;
    Ok(())
}
