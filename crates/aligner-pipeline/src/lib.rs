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
