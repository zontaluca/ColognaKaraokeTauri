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

    // demucs-rs writes stems: drums.wav, bass.wav, other.wav, vocals.wav
    // Build instrumental by mixing non-vocal stems.
    let drums = stems_dir.join("drums.wav");
    let bass = stems_dir.join("bass.wav");
    let other = stems_dir.join("other.wav");

    if !drums.exists() || !bass.exists() || !other.exists() {
        return Err(format!(
            "Expected stems not found in {}",
            stems_dir.display()
        ));
    }

    let instrumental_path = output_dir.join("instrumental.wav");
    mix_wavs(&[&drums, &bass, &other], &instrumental_path)
        .map_err(|e| format!("Mix failed: {}", e))?;

    // Save vocals stem before cleanup
    let vocals_stem = stems_dir.join("vocals.wav");
    if vocals_stem.exists() {
        let _ = std::fs::copy(&vocals_stem, output_dir.join("vocals.wav"));
    }

    // Cleanup stems dir
    let _ = std::fs::remove_dir_all(&stems_dir);

    on_progress("Separation complete", 1.0);
    Ok(instrumental_path)
}

/// Read PCM WAV files of identical format, sum their samples, normalize
/// to prevent clipping, and write a single WAV.
fn mix_wavs(inputs: &[&Path], out: &Path) -> Result<(), String> {
    use std::io::BufReader;

    if inputs.is_empty() {
        return Err("no inputs".into());
    }

    // Open all readers
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

    // Read all samples as f32
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
        let mut s = 0.0_f32;
        for v in &all_samples {
            s += v[i];
        }
        mixed.push(s);
    }

    // Normalize if peak > 1.0
    let peak = mixed.iter().cloned().map(f32::abs).fold(0.0_f32, f32::max);
    if peak > 1.0 {
        for s in mixed.iter_mut() {
            *s /= peak;
        }
    }

    let out_spec = hound::WavSpec {
        channels: spec.channels,
        sample_rate: spec.sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(out, out_spec).map_err(|e| e.to_string())?;
    for s in mixed {
        let clamped = s.clamp(-1.0, 1.0);
        let v = (clamped * i16::MAX as f32) as i16;
        writer.write_sample(v).map_err(|e| e.to_string())?;
    }
    writer.finalize().map_err(|e| e.to_string())?;
    Ok(())
}
