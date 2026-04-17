use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use pitch_detection::detector::yin::YINDetector;
use pitch_detection::detector::PitchDetector;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::audio::load_wav_mono;
use crate::recorder::MicBufferState;

const WINDOW_SAMPLES: usize = 1024;
const HOP_MS: u64 = 10;
const POWER_THRESHOLD: f32 = 5.0;
const CLARITY_THRESHOLD: f32 = 0.7;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PitchPoint {
    pub time_ms: u64,
    pub hz: f32,
}

pub fn hz_to_midi(hz: f32) -> Option<f32> {
    if hz <= 0.0 { None } else { Some(69.0 + 12.0 * (hz / 440.0).log2()) }
}

/// Precompute reference pitch contour from vocals.wav → pitch.json.
pub async fn precompute_reference_pitch(dir: &Path) -> Result<(), String> {
    let vocals = dir.join("vocals.wav");
    if !vocals.exists() {
        return Err("vocals.wav not found".into());
    }
    let out_path = dir.join("pitch.json");
    if out_path.exists() {
        return Ok(());
    }
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let (samples, sr) = load_wav_mono(&dir.join("vocals.wav"))?;
        let points = analyze_contour(&samples, sr);
        let json = serde_json::to_string(&points).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("pitch.json"), json).map_err(|e| e.to_string())?;
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(())
}

fn analyze_contour(samples: &[f32], sample_rate: u32) -> Vec<PitchPoint> {
    let hop = (sample_rate as u64 * HOP_MS / 1000) as usize;
    let win = WINDOW_SAMPLES;
    if samples.len() < win { return Vec::new(); }
    let mut detector = YINDetector::new(win, win / 2);
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos + win <= samples.len() {
        let slice = &samples[pos..pos + win];
        if let Some(p) = detector.get_pitch(slice, sample_rate as usize, POWER_THRESHOLD, CLARITY_THRESHOLD) {
            out.push(PitchPoint {
                time_ms: (pos as u64 * 1000) / sample_rate as u64,
                hz: p.frequency,
            });
        }
        pos += hop;
    }
    out
}

/// Active pitch-analyzer worker state.
#[derive(Default)]
pub struct PitchRuntime {
    pub reference: Vec<PitchPoint>,
    pub active: bool,
    pub song_start_epoch_ms: u64,
    pub word_boundaries: Vec<(u64, u64)>, // (start_ms, end_ms) per word in song timeline
}

pub type PitchState = Arc<Mutex<PitchRuntime>>;

pub fn init(app: &AppHandle) {
    let state: PitchState = Arc::new(Mutex::new(PitchRuntime::default()));
    app.manage(state);
}

#[tauri::command]
pub async fn pitch_start(
    app: AppHandle,
    song_dir: String,
    state: State<'_, PitchState>,
    mic: State<'_, MicBufferState>,
) -> Result<(), String> {
    // Load reference pitch + words
    let dir = std::path::PathBuf::from(&song_dir);
    let pitch_json = dir.join("pitch.json");
    let reference: Vec<PitchPoint> = if pitch_json.exists() {
        let s = std::fs::read_to_string(&pitch_json).map_err(|e| e.to_string())?;
        serde_json::from_str(&s).map_err(|e| e.to_string())?
    } else {
        Vec::new()
    };
    let words_json = dir.join("words.json");
    let word_boundaries: Vec<(u64, u64)> = if words_json.exists() {
        let s = std::fs::read_to_string(&words_json).map_err(|e| e.to_string())?;
        let v: serde_json::Value = serde_json::from_str(&s).map_err(|e| e.to_string())?;
        v.as_array()
            .map(|arr| {
                arr.iter()
                    .map(|w| {
                        (
                            w.get("start_ms").and_then(|x| x.as_u64()).unwrap_or(0),
                            w.get("end_ms").and_then(|x| x.as_u64()).unwrap_or(0),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    {
        let mut s = state.lock();
        s.reference = reference;
        s.word_boundaries = word_boundaries;
        s.active = true;
        s.song_start_epoch_ms = now_ms();
    }

    let state_clone = state.inner().clone();
    let mic_clone = mic.inner().clone();
    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        run_analyzer(app_clone, state_clone, mic_clone).await;
    });
    Ok(())
}

#[tauri::command]
pub fn pitch_stop(state: State<'_, PitchState>) -> Result<(), String> {
    state.lock().active = false;
    Ok(())
}

async fn run_analyzer(app: AppHandle, state: PitchState, mic: MicBufferState) {
    let mut last_word_reported: i64 = -1;
    loop {
        {
            if !state.lock().active { break; }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Snapshot mic
        let (samples, sr) = {
            let b = mic.lock();
            (b.samples.clone(), b.sample_rate)
        };
        if samples.is_empty() || sr == 0 { continue; }

        // Take last ~200ms for pitch
        let take_n = ((sr as usize) / 5).min(samples.len());
        if take_n < WINDOW_SAMPLES { continue; }
        let window = &samples[samples.len() - take_n..];

        // Median pitch over small hops
        let mut hz_vals = Vec::new();
        let mut detector = YINDetector::new(WINDOW_SAMPLES, WINDOW_SAMPLES / 2);
        let mut i = 0;
        while i + WINDOW_SAMPLES <= window.len() {
            if let Some(p) = detector.get_pitch(
                &window[i..i + WINDOW_SAMPLES],
                sr as usize,
                POWER_THRESHOLD,
                CLARITY_THRESHOLD,
            ) {
                hz_vals.push(p.frequency);
            }
            i += WINDOW_SAMPLES / 2;
        }
        if hz_vals.is_empty() { continue; }
        hz_vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let sung_hz = hz_vals[hz_vals.len() / 2];
        let sung_midi = match hz_to_midi(sung_hz) { Some(m) => m, None => continue };

        // Figure out current word from song timeline
        let (elapsed_ms, reference, boundaries) = {
            let s = state.lock();
            (
                now_ms().saturating_sub(s.song_start_epoch_ms),
                s.reference.clone(),
                s.word_boundaries.clone(),
            )
        };
        let word_idx = boundaries
            .iter()
            .position(|(st, en)| elapsed_ms >= *st && elapsed_ms < *en);
        let Some(word_idx) = word_idx else { continue };
        if word_idx as i64 == last_word_reported { /* allow re-scoring */ }

        let (ws, we) = boundaries[word_idx];
        let ref_hz = median_hz_in_range(&reference, ws, we);
        let Some(ref_hz) = ref_hz else { continue };
        let ref_midi = match hz_to_midi(ref_hz) { Some(m) => m, None => continue };
        let diff = (sung_midi - ref_midi).abs();

        // Fold octave errors
        let folded = diff - 12.0 * (diff / 12.0).floor();
        let folded = folded.min(12.0 - folded);

        let status = if folded <= 1.0 {
            "hit"
        } else if folded <= 2.0 {
            "partial"
        } else {
            "miss"
        };

        let _ = app.emit(
            "karaoke://score-tick",
            serde_json::json!({
                "word_idx": word_idx,
                "status": status,
                "note_diff": folded,
            }),
        );
        last_word_reported = word_idx as i64;
    }
}

fn median_hz_in_range(ref_points: &[PitchPoint], start_ms: u64, end_ms: u64) -> Option<f32> {
    let mut hz: Vec<f32> = ref_points
        .iter()
        .filter(|p| p.time_ms >= start_ms && p.time_ms < end_ms && p.hz > 0.0)
        .map(|p| p.hz)
        .collect();
    if hz.is_empty() { return None; }
    hz.sort_by(|a, b| a.partial_cmp(b).unwrap());
    Some(hz[hz.len() / 2])
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
