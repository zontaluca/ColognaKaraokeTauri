use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use aligner_pipeline::AudioBuffer;
use parking_lot::Mutex;
use pitch_detection::detector::yin::YINDetector;
use pitch_detection::detector::PitchDetector;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::recorder::MicBufferState;

const WINDOW_SAMPLES: usize = 1024;
const HOP_MS: u64 = 10;
const POWER_THRESHOLD: f32 = 5.0;
const CLARITY_THRESHOLD: f32 = 0.7;
const ANALYZER_TICK: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PitchPoint {
    pub time_ms: u64,
    pub hz: f32,
}

pub fn hz_to_midi(hz: f32) -> Option<f32> {
    if hz <= 0.0 { None } else { Some(69.0 + 12.0 * (hz / 440.0).log2()) }
}

/// True when the song already has a cached reference contour (pitch.json).
pub fn has_reference_pitch(dir: &Path) -> bool {
    dir.join("pitch.json").exists()
}

/// Compute the reference pitch contour from already-decoded vocals → pitch.json.
/// CPU-bound: call it from a blocking thread.
pub fn write_reference_pitch(dir: &Path, vocals: &AudioBuffer) -> Result<(), String> {
    let points = analyze_contour(&vocals.samples, vocals.sample_rate);
    let json = serde_json::to_string(&points).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("pitch.json"), json).map_err(|e| e.to_string())
}

fn analyze_contour(samples: &[f32], sample_rate: u32) -> Vec<PitchPoint> {
    let hop = (sample_rate as u64 * HOP_MS / 1000) as usize;
    let win = WINDOW_SAMPLES;
    if samples.len() < win || hop == 0 { return Vec::new(); }
    let mut detector = YINDetector::new(win, win / 2);
    let mut out = Vec::with_capacity((samples.len() - win) / hop + 1);
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

/// One scoring target: a word of words.json with its reference note.
#[derive(Debug, Clone, Copy)]
pub struct WordTarget {
    pub start_ms: u64,
    pub end_ms: u64,
    /// Median reference note over the word, `None` when the contour has no
    /// voiced frame inside it (the word is then never scored).
    pub ref_midi: Option<f32>,
}

/// Active pitch-analyzer worker state.
#[derive(Default)]
pub struct PitchRuntime {
    pub active: bool,
    pub song_start_epoch_ms: u64,
    /// Bumped on every `pitch_start`: an analyzer thread exits as soon as it
    /// sees a newer generation, so a second start never leaves two running.
    pub generation: u64,
    /// words.json in order (index = `word_idx` sent to the frontend).
    pub words: Vec<WordTarget>,
}

pub type PitchState = Arc<Mutex<PitchRuntime>>;

pub fn init(app: &AppHandle) {
    let state: PitchState = Arc::new(Mutex::new(PitchRuntime::default()));
    app.manage(state);
}

#[tauri::command(async)]
pub fn pitch_start(
    app: AppHandle,
    song_dir: String,
    state: State<'_, PitchState>,
    mic: State<'_, MicBufferState>,
) -> Result<(), String> {
    // Load reference pitch + words
    let dir = std::path::PathBuf::from(&song_dir);
    let pitch_json = dir.join("pitch.json");
    if !pitch_json.exists() {
        return Err("pitch.json not found — elabora la canzone prima di usare il challenge".into());
    }
    let reference: Vec<PitchPoint> = {
        let s = std::fs::read_to_string(&pitch_json).map_err(|e| e.to_string())?;
        serde_json::from_str(&s).map_err(|e| e.to_string())?
    };
    let words_json = dir.join("words.json");
    if !words_json.exists() {
        return Err("words.json not found — allineamento parole non disponibile per questa canzone".into());
    }
    let word_boundaries: Vec<(u64, u64)> = {
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
    };
    // Reference notes never change during a session: resolve them once here
    // instead of re-filtering the whole contour on every analyzer tick.
    let words: Vec<WordTarget> = word_boundaries
        .iter()
        .map(|&(start_ms, end_ms)| WordTarget {
            start_ms,
            end_ms,
            ref_midi: median_hz_in_range(&reference, start_ms, end_ms).and_then(hz_to_midi),
        })
        .collect();

    let generation = {
        let mut s = state.lock();
        s.words = words;
        s.active = true;
        s.song_start_epoch_ms = now_ms();
        s.generation = s.generation.wrapping_add(1);
        s.generation
    };

    let state_clone = state.inner().clone();
    let mic_clone = mic.inner().clone();
    // YIN keeps !Send scratch buffers, and the loop is CPU work: give it its own
    // thread rather than an async task.
    std::thread::Builder::new()
        .name("pitch-analyzer".into())
        .spawn(move || run_analyzer(app, state_clone, mic_clone, generation))
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn pitch_stop(state: State<'_, PitchState>) -> Result<(), String> {
    state.lock().active = false;
    Ok(())
}

/// Sync the analyzer clock to actual audio playback position.
/// Call this from JS on `timeupdate` to keep wall-clock in step with audio.currentTime.
#[tauri::command]
pub fn pitch_sync(state: State<'_, PitchState>, elapsed_ms: u64) -> Result<(), String> {
    let mut s = state.lock();
    s.song_start_epoch_ms = now_ms().saturating_sub(elapsed_ms);
    Ok(())
}

fn run_analyzer(app: AppHandle, state: PitchState, mic: MicBufferState, generation: u64) {
    let mut detector = YINDetector::new(WINDOW_SAMPLES, WINDOW_SAMPLES / 2);
    let mut window: Vec<f32> = Vec::new();
    let mut hz_vals: Vec<f32> = Vec::new();
    loop {
        {
            let s = state.lock();
            if !s.active || s.generation != generation { break; }
        }
        std::thread::sleep(ANALYZER_TICK);

        // Snapshot only the last ~200 ms of mic audio (not the whole ring buffer)
        let sr = {
            let b = mic.lock();
            let take_n = ((b.sample_rate as usize) / 5).min(b.samples.len());
            window.clear();
            window.extend_from_slice(&b.samples[b.samples.len() - take_n..]);
            b.sample_rate
        };
        if window.len() < WINDOW_SAMPLES || sr == 0 { continue; }

        // Median pitch over small hops
        hz_vals.clear();
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
        hz_vals.sort_by(|a, b| a.total_cmp(b));
        let sung_hz = hz_vals[hz_vals.len() / 2];
        let sung_midi = match hz_to_midi(sung_hz) { Some(m) => m, None => continue };

        // Figure out current word from song timeline
        let target = {
            let s = state.lock();
            let elapsed_ms = now_ms().saturating_sub(s.song_start_epoch_ms);
            current_word(&s.words, elapsed_ms)
        };
        let Some((word_idx, ref_midi)) = target else { continue };
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
    }
}

/// Word being sung at `elapsed_ms` with its reference note. Word starts are
/// monotonic (aligner guarantee), so a binary search replaces the linear scan.
fn current_word(words: &[WordTarget], elapsed_ms: u64) -> Option<(usize, f32)> {
    let idx = words.partition_point(|w| w.start_ms <= elapsed_ms).checked_sub(1)?;
    let w = words[idx];
    if elapsed_ms >= w.end_ms {
        return None;
    }
    w.ref_midi.map(|m| (idx, m))
}

/// Median reference frequency in `[start_ms, end_ms)`. The contour is sorted by
/// time, so the range is located by binary search.
fn median_hz_in_range(ref_points: &[PitchPoint], start_ms: u64, end_ms: u64) -> Option<f32> {
    let lo = ref_points.partition_point(|p| p.time_ms < start_ms);
    let hi = ref_points.partition_point(|p| p.time_ms < end_ms);
    let mut hz: Vec<f32> = ref_points
        .get(lo..hi.max(lo))?
        .iter()
        .filter(|p| p.hz > 0.0)
        .map(|p| p.hz)
        .collect();
    if hz.is_empty() { return None; }
    hz.sort_by(|a, b| a.total_cmp(b));
    Some(hz[hz.len() / 2])
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(time_ms: u64, hz: f32) -> PitchPoint {
        PitchPoint { time_ms, hz }
    }

    #[test]
    fn median_uses_only_points_in_range() {
        let pts = vec![point(0, 100.0), point(10, 200.0), point(20, 300.0), point(30, 400.0)];
        assert_eq!(median_hz_in_range(&pts, 10, 30), Some(300.0));
        assert_eq!(median_hz_in_range(&pts, 40, 50), None);
    }

    #[test]
    fn current_word_finds_containing_word() {
        let words = vec![
            WordTarget { start_ms: 0, end_ms: 100, ref_midi: Some(60.0) },
            WordTarget { start_ms: 200, end_ms: 300, ref_midi: Some(62.0) },
            WordTarget { start_ms: 300, end_ms: 400, ref_midi: None },
        ];
        assert_eq!(current_word(&words, 50), Some((0, 60.0)));
        assert_eq!(current_word(&words, 150), None);
        assert_eq!(current_word(&words, 250), Some((1, 62.0)));
        assert_eq!(current_word(&words, 350), None);
    }
}
