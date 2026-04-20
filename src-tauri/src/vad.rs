use std::path::Path;

use serde::{Deserialize, Serialize};

const SAMPLE_RATE: u32 = 16_000;
const FRAME_MS: u64 = 20;
const FRAME_SAMPLES: usize = (SAMPLE_RATE as u64 * FRAME_MS / 1000) as usize;
const SPEECH_OFFSET_DB: f32 = 8.0;
const ENTER_FRAMES: usize = 3;
const EXIT_FRAMES: usize = 8;
const MERGE_GAP_MS: u64 = 250;
const MIN_REGION_MS: u64 = 150;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VocalIntervals {
    pub regions: Vec<(u64, u64)>,
}

impl VocalIntervals {
    pub fn total_vocal_ms(&self) -> u64 {
        self.regions.iter().map(|(s, e)| e.saturating_sub(*s)).sum()
    }

    pub fn first_vocal_in_window(&self, start_ms: u64, end_ms: u64) -> Option<u64> {
        for (s, e) in &self.regions {
            if *e <= start_ms {
                continue;
            }
            if *s >= end_ms {
                break;
            }
            return Some((*s).max(start_ms));
        }
        None
    }
}

fn rms_dbfs(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return -120.0;
    }
    let sum_sq: f32 = frame.iter().map(|s| s * s).sum();
    let rms = (sum_sq / frame.len() as f32).sqrt();
    if rms <= 1e-7 {
        -120.0
    } else {
        20.0 * rms.log10()
    }
}

fn percentile(values: &mut Vec<f32>, p: f32) -> f32 {
    if values.is_empty() {
        return -120.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((values.len() - 1) as f32 * p).round() as usize;
    values[idx]
}

pub fn detect_vocals(vocals_wav: &Path) -> Result<VocalIntervals, String> {
    let samples = crate::audio::load_wav_mono_16k(vocals_wav)?;
    if samples.is_empty() {
        return Ok(VocalIntervals::default());
    }

    let frame_count = samples.len() / FRAME_SAMPLES;
    let mut levels: Vec<f32> = Vec::with_capacity(frame_count);
    for i in 0..frame_count {
        let start = i * FRAME_SAMPLES;
        let end = start + FRAME_SAMPLES;
        levels.push(rms_dbfs(&samples[start..end]));
    }

    let mut sorted = levels.clone();
    let noise_floor = percentile(&mut sorted, 0.10);
    let threshold = noise_floor + SPEECH_OFFSET_DB;

    let mut regions: Vec<(u64, u64)> = Vec::new();
    let mut in_region = false;
    let mut above_run = 0usize;
    let mut below_run = 0usize;
    let mut region_start_frame = 0usize;

    for (idx, lvl) in levels.iter().enumerate() {
        if *lvl > threshold {
            above_run += 1;
            below_run = 0;
            if !in_region && above_run >= ENTER_FRAMES {
                in_region = true;
                region_start_frame = idx + 1 - ENTER_FRAMES;
            }
        } else {
            below_run += 1;
            above_run = 0;
            if in_region && below_run >= EXIT_FRAMES {
                in_region = false;
                let region_end_frame = idx + 1 - EXIT_FRAMES;
                let s = region_start_frame as u64 * FRAME_MS;
                let e = region_end_frame as u64 * FRAME_MS;
                if e > s {
                    regions.push((s, e));
                }
            }
        }
    }
    if in_region {
        let s = region_start_frame as u64 * FRAME_MS;
        let e = frame_count as u64 * FRAME_MS;
        if e > s {
            regions.push((s, e));
        }
    }

    let mut merged: Vec<(u64, u64)> = Vec::new();
    for r in regions {
        if let Some(last) = merged.last_mut() {
            if r.0.saturating_sub(last.1) <= MERGE_GAP_MS {
                last.1 = r.1;
                continue;
            }
        }
        merged.push(r);
    }

    merged.retain(|(s, e)| e.saturating_sub(*s) >= MIN_REGION_MS);

    Ok(VocalIntervals { regions: merged })
}

pub fn save_to_disk(iv: &VocalIntervals, dir: &Path) -> Result<(), String> {
    let path = dir.join("vad.json");
    let s = serde_json::to_string_pretty(iv).map_err(|e| e.to_string())?;
    std::fs::write(&path, s).map_err(|e| e.to_string())
}

pub fn load_from_disk(dir: &Path) -> Option<VocalIntervals> {
    let path = dir.join("vad.json");
    let s = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&s).ok()
}

pub fn load_or_compute(dir: &Path) -> Result<VocalIntervals, String> {
    if let Some(iv) = load_from_disk(dir) {
        return Ok(iv);
    }
    let vocals = dir.join("vocals.wav");
    if !vocals.exists() {
        return Err("vocals.wav not found".into());
    }
    let iv = detect_vocals(&vocals)?;
    save_to_disk(&iv, dir)?;
    Ok(iv)
}
