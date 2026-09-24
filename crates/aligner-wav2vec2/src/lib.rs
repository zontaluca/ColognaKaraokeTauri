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
        self.align_owned(
            AudioBuffer {
                samples: vocals.samples.clone(),
                sample_rate: vocals.sample_rate,
            },
            lyrics,
            time_offset_sec,
        )
    }

    /// Same as [`Self::align`], but takes ownership of the buffer so
    /// preprocessing runs in place instead of on a full copy of the song.
    pub fn align_owned(
        &mut self,
        vocals: AudioBuffer,
        lyrics: &str,
        time_offset_sec: f64,
    ) -> Result<Vec<AlignedWord>, AlignError> {
        let emissions = self.emissions(vocals, time_offset_sec)?;
        self.align_emissions(&emissions, lyrics, None)
    }

    /// Run the acoustic model once over `vocals` (16 kHz mono, consumed) and
    /// keep the per-frame log-probabilities. Several texts or time windows can
    /// then be aligned against them with [`Self::align_emissions`] without
    /// running inference again.
    pub fn emissions(
        &mut self,
        vocals: AudioBuffer,
        time_offset_sec: f64,
    ) -> Result<Emissions, AlignError> {
        if vocals.sample_rate != TARGET_SAMPLE_RATE {
            return Err(AlignError::BadSampleRate {
                expected: TARGET_SAMPLE_RATE,
                got: vocals.sample_rate,
            });
        }
        if vocals.samples.is_empty() {
            return Ok(Emissions {
                log_probs: Array2::zeros((0, self.vocab.vocab_size.max(1) as usize)),
                time_offset_sec,
            });
        }

        let mut work = vocals.samples;
        highpass_biquad(&mut work, vocals.sample_rate, self.config.highpass_hz);
        rms_normalize(&mut work, self.config.rms_target_dbfs);

        let logits = self.chunked_infer(&work)?;
        Ok(Emissions {
            log_probs: log_softmax_rows(logits.view()),
            time_offset_sec,
        })
    }

    /// Align `lyrics` against precomputed emissions. With `window` =
    /// `Some((start_sec, end_sec))` (song time) only the frames inside that
    /// range are used, which confines the text to that part of the song.
    pub fn align_emissions(
        &self,
        emissions: &Emissions,
        lyrics: &str,
        window: Option<(f64, f64)>,
    ) -> Result<Vec<AlignedWord>, AlignError> {
        let total = emissions.log_probs.shape()[0];
        let (f0, f1) = match window {
            None => (0, total),
            Some((start_sec, end_sec)) => (
                emissions.frame_at(start_sec).min(total),
                emissions.frame_at(end_sec).min(total),
            ),
        };
        if f1 <= f0 {
            return Err(AlignError::Ctc("empty alignment window".into()));
        }
        let view = emissions.log_probs.slice(ndarray::s![f0..f1, ..]);
        let offset = emissions.time_offset_sec + f0 as f64 * FRAME_MS / 1000.0;
        align_log_probs(view, lyrics, &self.vocab, offset)
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
        // Collect per-chunk logits and concatenate once at the end; growing an
        // accumulator chunk by chunk would copy the whole matrix every time.
        let mut parts: Vec<Array2<f32>> = Vec::with_capacity(total_chunks);
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
            parts.push(logits);
        }
        match parts.len() {
            0 => Err(AlignError::Inference("no chunks produced any frames".into())),
            1 => Ok(parts.pop().expect("one part")),
            _ => {
                let views: Vec<_> = parts.iter().map(|p| p.view()).collect();
                ndarray::concatenate(ndarray::Axis(0), &views)
                    .map_err(|e| AlignError::Inference(format!("concat logits: {}", e)))
            }
        }
    }
}

/// Per-frame log-probabilities from one inference pass, plus the song time of
/// frame 0.
pub struct Emissions {
    pub log_probs: Array2<f32>,
    pub time_offset_sec: f64,
}

impl Emissions {
    /// Number of frames.
    pub fn frames(&self) -> usize {
        self.log_probs.shape()[0]
    }

    /// Song time just past the last frame.
    pub fn end_sec(&self) -> f64 {
        self.time_offset_sec + self.frames() as f64 * FRAME_MS / 1000.0
    }

    /// Frame index covering song time `sec` (clamped at 0).
    pub fn frame_at(&self, sec: f64) -> usize {
        (((sec - self.time_offset_sec) * 1000.0 / FRAME_MS).max(0.0)).round() as usize
    }
}

/// CTC-align `lyrics` against `log_probs` ([T, V] log-softmax) whose first
/// frame sits at `time_offset_sec`. Pure function: no model involved.
pub fn align_log_probs(
    log_probs: ndarray::ArrayView2<f32>,
    lyrics: &str,
    vocab: &Vocab,
    time_offset_sec: f64,
) -> Result<Vec<AlignedWord>, AlignError> {
    let target = lyrics_to_target_ids(lyrics, vocab);
    if target.words.is_empty() {
        return Ok(Vec::new());
    }

    let spans = forced_align(log_probs, &target.ids, vocab.pad_id)?;

    let mut out = Vec::with_capacity(target.words.len());
    for word in &target.words {
        let span = aggregate_word_span(&spans, &word.token_range);
        let start_s = time_offset_sec + (span.start as f64) * FRAME_MS / 1000.0;
        let end_s = time_offset_sec + ((span.end + 1) as f64) * FRAME_MS / 1000.0;
        let conf = mean_emission_prob(log_probs, &target.ids, &word.token_range, &spans);
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

fn aggregate_word_span(spans: &[TokenSpan], range: &std::ops::Range<usize>) -> TokenSpan {
    let slice = &spans[range.clone()];
    let start = slice.iter().map(|s| s.start).min().unwrap_or(0);
    let end = slice.iter().map(|s| s.end).max().unwrap_or(start);
    TokenSpan { start, end }
}

fn mean_emission_prob(
    log_probs: ndarray::ArrayView2<f32>,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_vocab() -> Vocab {
        let json = br#"{"<pad>":0,"<s>":1,"</s>":2,"<unk>":3,"|":4,"a":5,"b":6}"#;
        Vocab::from_bytes(json).unwrap()
    }

    /// Log-probs where each frame strongly predicts one id.
    fn peaked(frames: &[u32], vocab: usize) -> Array2<f32> {
        let mut data = Vec::with_capacity(frames.len() * vocab);
        for &f in frames {
            data.extend((0..vocab).map(|i| if i as u32 == f { -0.01 } else { -8.0 }));
        }
        Array2::from_shape_vec((frames.len(), vocab), data).unwrap()
    }

    #[test]
    fn aligns_words_with_time_offset_and_confidence() {
        let vocab = fake_vocab();
        // "a" at frames 1-2, delimiter at 3, "b" at frames 5-6 (blank = pad = 0)
        let lp = peaked(&[0, 5, 5, 4, 0, 6, 6, 0], 7);
        let words = align_log_probs(lp.view(), "a b", &vocab, 10.0).unwrap();
        assert_eq!(words.len(), 2);
        assert!((words[0].start - 10.02).abs() < 1e-9, "{}", words[0].start);
        assert!((words[1].start - 10.10).abs() < 1e-9, "{}", words[1].start);
        assert!(words[0].confidence > 0.9);
    }

    #[test]
    fn window_maps_frames_to_song_time() {
        let em = Emissions { log_probs: Array2::zeros((500, 7)), time_offset_sec: 2.0 };
        assert_eq!(em.frame_at(2.0), 0);
        assert_eq!(em.frame_at(3.0), 50);
        assert_eq!(em.frame_at(0.0), 0);
        assert!((em.end_sec() - 12.0).abs() < 1e-9);
    }
}
