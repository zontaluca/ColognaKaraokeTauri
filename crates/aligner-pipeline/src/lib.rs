use serde::{Deserialize, Serialize};

/// Mono PCM audio at a known sample rate. f32 samples in [-1.0, 1.0].
pub struct AudioBuffer {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

/// Contiguous region where voice is detected. Times in seconds from song start.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceSegment {
    pub start: f64,
    pub end: f64,
}

/// Single aligned word. `confidence` in [0.0, 1.0]; below 0.3 flagged in UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlignedWord {
    /// Original form as given by the user (case + punctuation preserved).
    pub word: String,
    /// Lowercased, punctuation-stripped — used for alignment only.
    pub normalized: String,
    pub start: f64,
    pub end: f64,
    pub confidence: f32,
}

/// One chunk of the final timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum TimelineEntry {
    Music { start: f64, end: f64 },
    Vocal { start: f64, end: f64, words: Vec<AlignedWord> },
}

pub type Timeline = Vec<TimelineEntry>;

/// Progress reported by the pipeline. `fraction` is in [0.0, 1.0] across the whole job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Progress {
    pub stage: Stage,
    pub fraction: f32,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Stage {
    Decoding,
    SeparatingStems,
    DetectingVoice,
    Aligning,
    BuildingTimeline,
    Done,
}

/// First and last seconds where vocal RMS exceeds 15% of the 95th-percentile
/// RMS. Operates on already-separated vocals so the only meaningful energy
/// is singing. Returned tuple is `(onset, offset)` clamped to the buffer.
///
/// Used by aligner backends to crop instrumental intros/outros before
/// inference and to avoid placing late words in post-vocal silence.
pub fn detect_vocal_range(samples: &[f32], sample_rate: u32) -> (f64, f64) {
    let win_samples = (sample_rate as usize / 10).max(1); // 100 ms windows
    let rms: Vec<f32> = samples
        .chunks(win_samples)
        .map(|w| {
            let s: f32 = w.iter().map(|x| x * x).sum();
            (s / w.len() as f32).sqrt()
        })
        .collect();

    if rms.is_empty() {
        return (0.0, samples.len() as f64 / sample_rate as f64);
    }

    let mut sorted = rms.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p95 = sorted[((sorted.len() as f64 * 0.95) as usize).min(sorted.len() - 1)];
    let thr = p95 * 0.15;

    // Smooth with a 5-window (500 ms) majority filter so isolated bleed
    // spikes don't register as onset.
    let n = rms.len();
    let mut smoothed = vec![false; n];
    let win = 5usize;
    let half = win / 2;
    for i in 0..n {
        let lo = i.saturating_sub(half);
        let hi = (i + half + 1).min(n);
        smoothed[i] = rms[lo..hi].iter().filter(|&&v| v > thr).count() >= win / 2 + 1;
    }

    let first = smoothed.iter().position(|&v| v).unwrap_or(0);
    let last = smoothed.iter().rposition(|&v| v).unwrap_or(n - 1);

    let win_sec = win_samples as f64 / sample_rate as f64;
    let onset = first as f64 * win_sec;
    let offset = ((last + 1) as f64 * win_sec).min(samples.len() as f64 / sample_rate as f64);
    (onset, offset)
}
