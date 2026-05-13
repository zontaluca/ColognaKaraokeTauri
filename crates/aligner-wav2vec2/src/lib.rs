//! Word-level forced alignment via wav2vec2 CTC.
//!
//! Pipeline (per song):
//!   1. Preprocess vocals (high-pass + RMS normalise).
//!   2. Run wav2vec2 ONNX → logits `[T, V]`.
//!   3. Log-softmax across vocab axis.
//!   4. Build CTC target id sequence from lyrics (char-level + `|` between
//!      words).
//!   5. Viterbi forced alignment → per-token frame spans.
//!   6. Aggregate char spans into word spans, convert to seconds.

pub mod audio;
pub mod ctc;
pub mod model;
pub mod text;
pub mod vocab;

use aligner_pipeline::{AlignedWord, AudioBuffer};
use ndarray::Array2;

use crate::audio::{highpass_biquad, rms_normalize};
use crate::ctc::{forced_align, TokenSpan};
use crate::model::{log_softmax_rows, resolve_model_paths, OnnxSession};
pub use crate::model::ModelSource;
use crate::text::lyrics_to_target_ids;
use crate::vocab::Vocab;

/// wav2vec2 receptive field stride at 16 kHz: 320 samples = 20 ms / frame.
pub const FRAME_MS: f64 = 20.0;
pub const SAMPLES_PER_FRAME: usize = 320;
pub const TARGET_SAMPLE_RATE: u32 = 16_000;

#[derive(Debug, thiserror::Error)]
pub enum AlignError {
    #[error("config: {0}")]
    Config(String),
    #[error("model load: {0}")]
    ModelLoad(String),
    #[error("inference: {0}")]
    Inference(String),
    #[error("ctc: {0}")]
    Ctc(String),
    #[error("invalid sample rate: expected {expected}, got {got}")]
    BadSampleRate { expected: u32, got: u32 },
}

#[derive(Debug, Clone)]
pub struct AlignmentConfig {
    pub source: ModelSource,
    /// ISO-639-1 language code. Currently only used for diagnostics; the
    /// caller is responsible for selecting a language-specific [`ModelSource`].
    pub language: String,
    pub highpass_hz: f32,
    pub rms_target_dbfs: f32,
    /// Maximum audio fed to a single ONNX `Session::run`. wav2vec2 attention
    /// is O(T²); processing a whole 4-minute song at once explodes memory and
    /// stalls the CPU EP. 20 s is safe on M-series Macs.
    pub chunk_seconds: f32,
}

impl Default for AlignmentConfig {
    fn default() -> Self {
        Self {
            source: ModelSource::LocalDir(default_local_dir("it")),
            language: "it".to_string(),
            highpass_hz: 80.0,
            rms_target_dbfs: -20.0,
            chunk_seconds: 20.0,
        }
    }
}

/// Default-to-look-here when the integrator hasn't picked a custom location.
pub fn default_local_dir(language: &str) -> std::path::PathBuf {
    dirs_next::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from(".cache"))
        .join("cologna-karaoke")
        .join("wav2vec2")
        .join(language)
}

pub struct Wav2vecAligner {
    config: AlignmentConfig,
    session: OnnxSession,
    vocab: Vocab,
}

impl Wav2vecAligner {
    /// Load weights + vocab from the configured source.
    pub async fn new(config: AlignmentConfig) -> Result<Self, AlignError> {
        let paths = resolve_model_paths(&config.source).await?;
        let vocab = Vocab::from_json(&paths.vocab)?;
        let session = OnnxSession::load(&paths.onnx)?;
        Ok(Self { config, session, vocab })
    }

    pub fn language(&self) -> &str {
        &self.config.language
    }

    pub fn vocab(&self) -> &Vocab {
        &self.vocab
    }

    /// Align `lyrics` to `vocals`. The input buffer must be 16 kHz mono and
    /// is consumed by-value (preprocessing mutates a working copy).
    ///
    /// Word-frame conversion uses [`FRAME_MS`]. If `time_offset_sec` is
    /// non-zero it is added to every output start/end — useful when the caller
    /// has cropped the audio to a vocal range and wants song-relative times.
    pub fn align(
        &mut self,
        vocals: &AudioBuffer,
        lyrics: &str,
        time_offset_sec: f64,
    ) -> Result<Vec<AlignedWord>, AlignError> {
        if vocals.sample_rate != TARGET_SAMPLE_RATE {
            return Err(AlignError::BadSampleRate {
                expected: TARGET_SAMPLE_RATE,
                got: vocals.sample_rate,
            });
        }
        if vocals.samples.is_empty() {
            return Ok(Vec::new());
        }

        let mut work = vocals.samples.clone();
        highpass_biquad(&mut work, vocals.sample_rate, self.config.highpass_hz);
        rms_normalize(&mut work, self.config.rms_target_dbfs);

        let logits = self.chunked_infer(&work)?;
        let log_probs = log_softmax_rows(logits.view());

        let target = lyrics_to_target_ids(lyrics, &self.vocab);
        if target.words.is_empty() {
            return Ok(Vec::new());
        }

        let spans = forced_align(log_probs.view(), &target.ids, self.vocab.pad_id)?;

        let mut out = Vec::with_capacity(target.words.len());
        for word in &target.words {
            let span = aggregate_word_span(&spans, &word.token_range);
            let start_s = time_offset_sec + (span.start as f64) * FRAME_MS / 1000.0;
            let end_s = time_offset_sec + ((span.end + 1) as f64) * FRAME_MS / 1000.0;
            let conf = mean_emission_prob(&log_probs, &target.ids, &word.token_range, &spans);
            let normalised: String = word
                .word
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '\'' || *c == '-')
                .flat_map(|c| c.to_lowercase())
                .collect();
            out.push(AlignedWord {
                word: word.word.clone(),
                normalized: normalised,
                start: start_s,
                end: end_s,
                confidence: conf,
            });
        }

        enforce_monotonic_starts(&mut out);
        Ok(out)
    }

    /// Run the ONNX model over `samples` in non-overlapping chunks, then
    /// concatenate the per-chunk logits along the time axis. Required because
    /// wav2vec2 self-attention is O(T²) and a single 4-minute pass exhausts
    /// memory on macOS CPU/CoreML backends.
    fn chunked_infer(&mut self, samples: &[f32]) -> Result<Array2<f32>, AlignError> {
        let chunk_samples = (self.config.chunk_seconds as f64
            * TARGET_SAMPLE_RATE as f64)
            .round() as usize;
        // wav2vec2 needs at least one full convolutional receptive field to
        // emit anything useful — drop chunks shorter than 0.5 s.
        let min_chunk = (TARGET_SAMPLE_RATE as usize) / 2;
        let chunk_samples = chunk_samples.max(min_chunk);

        if samples.len() <= chunk_samples {
            tracing::info!("[w2v] inference single-pass ({} samples)", samples.len());
            eprintln!("[w2v] inference single-pass ({} samples)", samples.len());
            return self.session.infer(samples);
        }

        let total_chunks = (samples.len() + chunk_samples - 1) / chunk_samples;
        eprintln!(
            "[w2v] chunked inference: {} samples in {} chunks of {:.1}s",
            samples.len(),
            total_chunks,
            self.config.chunk_seconds,
        );
        let mut acc: Option<Array2<f32>> = None;
        for (idx, start) in (0..samples.len()).step_by(chunk_samples).enumerate() {
            let end = (start + chunk_samples).min(samples.len());
            if end - start < min_chunk {
                eprintln!(
                    "[w2v] chunk {}/{} too short ({} samples) — skip",
                    idx + 1,
                    total_chunks,
                    end - start
                );
                continue;
            }
            let chunk = &samples[start..end];
            let logits = self.session.infer(chunk)?;
            eprintln!(
                "[w2v] chunk {}/{} ok ({}..{}, {} frames)",
                idx + 1,
                total_chunks,
                start,
                end,
                logits.shape()[0],
            );
            acc = Some(match acc {
                None => logits,
                Some(prev) => ndarray::concatenate![ndarray::Axis(0), prev, logits],
            });
        }
        acc.ok_or_else(|| AlignError::Inference("no chunks produced any frames".into()))
    }
}

fn aggregate_word_span(spans: &[TokenSpan], range: &std::ops::Range<usize>) -> TokenSpan {
    let slice = &spans[range.clone()];
    let start = slice.iter().map(|s| s.start).min().unwrap_or(0);
    let end = slice.iter().map(|s| s.end).max().unwrap_or(start);
    TokenSpan { start, end }
}

fn mean_emission_prob(
    log_probs: &Array2<f32>,
    targets: &[u32],
    range: &std::ops::Range<usize>,
    spans: &[TokenSpan],
) -> f32 {
    let mut count: usize = 0;
    let mut acc: f32 = 0.0;
    for tok_idx in range.clone() {
        let token_id = targets[tok_idx] as usize;
        let span = spans[tok_idx];
        for frame in span.start..=span.end {
            let lp = log_probs[[frame, token_id]];
            acc += lp.exp();
            count += 1;
        }
    }
    if count == 0 {
        0.0
    } else {
        (acc / count as f32).clamp(0.0, 1.0)
    }
}

fn enforce_monotonic_starts(words: &mut [AlignedWord]) {
    let mut cursor = 0.0_f64;
    for w in words.iter_mut() {
        if w.start < cursor {
            w.start = cursor;
        }
        if w.end < w.start + 0.02 {
            w.end = w.start + 0.02;
        }
        cursor = w.start;
    }
}
