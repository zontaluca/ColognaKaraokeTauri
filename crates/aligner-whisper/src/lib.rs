//! Forced word-level alignment via Whisper cross-attention + DTW.
//!
//! Does NOT call any external process. All inference runs in-process via candle.

pub mod dtw;
pub mod mel;
pub mod model;
pub mod normalize;

use aligner_pipeline::{AlignedWord, AudioBuffer};
use tracing::info;

use dtw::{dtw, path_to_token_spans};
use mel::{log_mel_spectrogram, make_chunks, FRAME_MS};
use model::{SpecialTokens, WhisperResources};
use normalize::{normalize_lyrics, normalize_word};

// ─── Public types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum WhisperModel {
    Small,
    Medium,
    LargeV3Turbo,
}

#[derive(Debug, Clone)]
pub struct AlignmentConfig {
    pub model: WhisperModel,
    /// ISO 639-1 language code, e.g. "it" or "en".
    pub language: String,
    pub chunk_seconds: f32,
    pub overlap_seconds: f32,
    /// Number of last decoder layers to average attention from.
    /// 0 means use all layers.
    pub attention_layers: usize,
}

impl Default for AlignmentConfig {
    fn default() -> Self {
        Self {
            model: WhisperModel::Medium,
            language: "it".to_string(),
            chunk_seconds: 30.0,
            overlap_seconds: 3.0,
            attention_layers: 0,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AlignError {
    #[error("model download failed: {0}")]
    ModelDownload(String),
    #[error("tokenization failed: {0}")]
    Tokenization(String),
    #[error("inference failed: {0}")]
    Inference(String),
    #[error("dtw failed: {0}")]
    Dtw(String),
    #[error("invalid sample rate: expected 16000, got {0}")]
    BadSampleRate(u32),
}

// ─── ForcedAligner ───────────────────────────────────────────────────────────

pub struct ForcedAligner {
    config: AlignmentConfig,
    resources: WhisperResources,
}

impl ForcedAligner {
    /// Load Whisper weights, downloading on first run.
    /// Call once and share via Arc.
    pub async fn new(config: AlignmentConfig) -> Result<Self, AlignError> {
        let cache_dir = platform_cache_dir();
        let resources = WhisperResources::load(&config.model, &cache_dir).await?;
        Ok(Self { config, resources })
    }

    /// Align `lyrics` to a 16 kHz mono `vocals` buffer.
    ///
    /// Returns one [`AlignedWord`] per whitespace-separated word in `lyrics`,
    /// preserving original order and casing. Words that cannot be reliably
    /// aligned receive a low confidence score but are still returned with a
    /// best-effort timestamp.
    pub fn align(
        &self,
        vocals: &AudioBuffer,
        lyrics: &str,
    ) -> Result<Vec<AlignedWord>, AlignError> {
        if vocals.sample_rate != 16_000 {
            return Err(AlignError::BadSampleRate(vocals.sample_rate));
        }

        let (orig_words, norm_words, _) = normalize_lyrics(lyrics);
        if orig_words.is_empty() {
            return Ok(vec![]);
        }

        // Tokenize each normalized word with a leading space (except the first)
        // to match how Whisper tokenizes transcribed text.
        let word_token_ids: Vec<Vec<u32>> = norm_words
            .iter()
            .enumerate()
            .map(|(i, w)| self.tokenize_single(w, i == 0))
            .collect::<Result<_, _>>()?;

        let flat_tokens: Vec<u32> = word_token_ids.iter().flatten().copied().collect();
        if flat_tokens.is_empty() {
            return Ok(build_empty_output(&orig_words));
        }

        // Build token → word index mapping.
        let token_to_word: Vec<usize> = word_token_ids
            .iter()
            .enumerate()
            .flat_map(|(wi, toks)| std::iter::repeat(wi).take(toks.len()))
            .collect();

        let special = SpecialTokens::for_language(
            &self.resources.tokenizer,
            &self.config.language,
        )?;
        let prefix = [
            special.sot,
            special.lang_id,
            special.transcribe,
            special.no_timestamps,
        ];

        let total_dur =
            vocals.samples.len() as f64 / vocals.sample_rate as f64;
        let chunks = make_chunks(
            &vocals.samples,
            self.config.chunk_seconds,
            self.config.overlap_seconds,
        );
        info!(
            "aligning {} words ({} tokens) across {} chunks, total {:.1}s",
            orig_words.len(),
            flat_tokens.len(),
            chunks.len(),
            total_dur
        );

        // word_times[i] = Some((start, end, confidence)) or None.
        let mut word_times: Vec<Option<(f64, f64, f32)>> =
            vec![None; orig_words.len()];

        for (chunk_samples, chunk_offset) in &chunks {
            let chunk_end =
                (chunk_offset + self.config.chunk_seconds as f64).min(total_dur);

            // Determine which content tokens belong to this chunk by proportional
            // split, with a small overlap buffer to avoid boundary drop-outs.
            let frac_s = (chunk_offset / total_dur).clamp(0.0, 1.0);
            let frac_e = (chunk_end / total_dur).clamp(0.0, 1.0);
            let buf = 5; // token overlap buffer
            let tok_s =
                ((frac_s * flat_tokens.len() as f64) as usize).saturating_sub(buf);
            let tok_e =
                ((frac_e * flat_tokens.len() as f64) as usize + buf)
                    .min(flat_tokens.len());

            if tok_s >= tok_e {
                continue;
            }

            let chunk_content = &flat_tokens[tok_s..tok_e];
            let mut seq: Vec<u32> = prefix.to_vec();
            seq.extend_from_slice(chunk_content);
            seq.push(special.eot);
            let prefix_len = prefix.len();

            // Encode mel.
            let mel = log_mel_spectrogram(chunk_samples, n_mels_for(&self.config.model));
            let encoder_out = self
                .encode_mel(&mel)
                .map_err(|e| AlignError::Inference(e.to_string()))?;
            let t_enc = encoder_out
                .dim(1)
                .map_err(|e| AlignError::Inference(e.to_string()))?;

            // Forced attention → [n_seq, t_enc].
            let attn = self
                .resources
                .decoder
                .forced_attention(
                    &encoder_out,
                    &seq,
                    self.config.attention_layers,
                    &self.resources.device,
                )
                .map_err(|e| AlignError::Inference(e.to_string()))?;

            let content_attn = &attn[prefix_len..seq.len() - 1];
            if content_attn.is_empty() || t_enc == 0 {
                continue;
            }

            // Apply a median filter to each attention row to smooth noise
            // (matches the preprocessing in openai-whisper's timing.py).
            let smoothed_full = median_filter_rows(content_attn, 7);
            let full_t_enc = smoothed_full.first().map(|r| r.len()).unwrap_or(0);

            // Use mel spectrogram energy (not attention, which is non-zero in silence
            // due to softmax) to find the last frame with actual speech content.
            // mel[m][f] values are near 0 for silence (log-mel clipped at max−8).
            let t_enc_active = {
                let mel_energy_per_enc_frame: Vec<f32> = (0..full_t_enc)
                    .map(|j| {
                        let mf = j * 2; // encoder frame j covers mel frames 2j, 2j+1
                        mel.iter()
                            .map(|row| row.get(mf).cloned().unwrap_or(0.0).max(0.0))
                            .sum::<f32>()
                    })
                    .collect();
                let max_e = mel_energy_per_enc_frame
                    .iter()
                    .cloned()
                    .fold(0.0f32, f32::max);
                let thr = max_e * 0.05;
                let last_speech = mel_energy_per_enc_frame
                    .iter()
                    .rposition(|&e| e > thr)
                    .unwrap_or(full_t_enc.saturating_sub(1));
                // Add a small buffer so the last word isn't cut off.
                (last_speech + 10).min(full_t_enc)
            };

            let smoothed: Vec<Vec<f32>> = smoothed_full
                .into_iter()
                .map(|row| row[..t_enc_active].to_vec())
                .collect();

            // Cost = 1 − attention; DTW → token spans in encoder-frame space.
            let cost: Vec<Vec<f32>> = smoothed
                .iter()
                .map(|row| row.iter().map(|&w| 1.0 - w).collect())
                .collect();
            let path = dtw(&cost);
            let spans = path_to_token_spans(&path, smoothed.len());

            // Map local token index → original word index → time.
            // For multi-token words keep the EARLIEST span start.
            for (local_idx, global_tok_idx) in (tok_s..tok_e).enumerate() {
                if local_idx >= spans.len() {
                    break;
                }
                let wi = token_to_word[global_tok_idx];
                let (fs, fe) = spans[local_idx];
                let span_dur = (fe + 1).saturating_sub(fs) as f64 * FRAME_MS / 1000.0;
                // Cross-attention peaks slightly past mid-word for Whisper Small.
                // 0.55 × span_dur back-shift was empirically optimal (better than 0.5
                // on the Italian TTS fixture where attention lags word onset by ~55%).
                let raw_start = chunk_offset + fs as f64 * FRAME_MS / 1000.0;
                let t_start = (raw_start - span_dur * 0.65).max(0.0);
                let t_end = chunk_offset + (fe + 1) as f64 * FRAME_MS / 1000.0;
                let conf = mean_attention(&smoothed, local_idx, fs, fe);

                match word_times[wi] {
                    None => word_times[wi] = Some((t_start, t_end, conf)),
                    Some((prev_s, _, _)) if t_start < prev_s => {
                        word_times[wi] = Some((t_start, t_end, conf));
                    }
                    _ => {}
                }
            }
        }

        // Fill gaps with linear interpolation between known anchors.
        fill_missing_times(&mut word_times, total_dur);

        // Enforce non-decreasing start times.
        enforce_monotonicity(&mut word_times);

        let result = orig_words
            .iter()
            .enumerate()
            .map(|(i, word)| {
                let (start, end, confidence) =
                    word_times[i].unwrap_or((0.0, 0.05, 0.1));
                AlignedWord {
                    word: word.clone(),
                    normalized: normalize_word(word).join(" "),
                    start,
                    end,
                    confidence,
                }
            })
            .collect();

        Ok(result)
    }

    // ─── Private helpers ─────────────────────────────────────────────────────

    fn tokenize_single(&self, word: &str, first: bool) -> Result<Vec<u32>, AlignError> {
        let text = if first {
            word.to_string()
        } else {
            format!(" {}", word)
        };
        let enc = self
            .resources
            .tokenizer
            .encode(text.as_str(), false)
            .map_err(|e| AlignError::Tokenization(e.to_string()))?;
        Ok(enc.get_ids().to_vec())
    }

    fn encode_mel(&self, mel: &[Vec<f32>]) -> candle_core::Result<candle_core::Tensor> {
        use candle_core::{DType, Tensor};
        use mel::ENCODER_FRAMES;

        let n_mels = mel.len();
        let target_frames = ENCODER_FRAMES * 2;

        let flat: Vec<f32> = mel
            .iter()
            .flat_map(|row| {
                let mut r = row.clone();
                r.resize(target_frames, 0.0);
                r
            })
            .collect();

        let t = Tensor::from_vec(flat, (1, n_mels, target_frames), &self.resources.device)?
            .to_dtype(DType::F32)?;

        self.resources.encoder.forward(&t)
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn n_mels_for(model: &WhisperModel) -> usize {
    match model {
        WhisperModel::LargeV3Turbo => 128,
        _ => 80,
    }
}

/// Apply a 1-D sliding median filter with the given window to each row.
/// Window is clamped to odd; edges are replicated.
fn median_filter_rows(attn: &[Vec<f32>], window: usize) -> Vec<Vec<f32>> {
    let w = (window | 1).max(1); // ensure odd
    let half = w / 2;
    attn.iter()
        .map(|row| {
            let n = row.len();
            (0..n)
                .map(|j| {
                    let lo = j.saturating_sub(half);
                    let hi = (j + half + 1).min(n);
                    let mut buf: Vec<f32> = row[lo..hi].to_vec();
                    buf.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                    buf[buf.len() / 2]
                })
                .collect()
        })
        .collect()
}

fn mean_attention(
    attn: &[Vec<f32>],
    tok_idx: usize,
    frame_s: usize,
    frame_e: usize,
) -> f32 {
    let row = match attn.get(tok_idx) {
        Some(r) => r,
        None => return 0.1,
    };
    let n = (frame_e + 1).saturating_sub(frame_s).max(1);
    let sum: f32 = (frame_s..=frame_e)
        .filter_map(|f| row.get(f).copied())
        .sum();
    (sum / n as f32).clamp(0.0, 1.0)
}

/// Fill None entries with linear interpolation between known (start, end) anchors.
fn fill_missing_times(times: &mut [Option<(f64, f64, f32)>], total_dur: f64) {
    let n = times.len();

    // Find first and last placed word; seed boundaries if none.
    let first_placed = times.iter().position(|t| t.is_some());
    let last_placed = times.iter().rposition(|t| t.is_some());

    let (fp, lp) = match (first_placed, last_placed) {
        (Some(f), Some(l)) => (f, l),
        _ => {
            // No alignment at all — distribute evenly.
            let step = total_dur / n as f64;
            for (i, t) in times.iter_mut().enumerate() {
                *t = Some((i as f64 * step, (i + 1) as f64 * step, 0.1));
            }
            return;
        }
    };

    // Fill prefix.
    if fp > 0 {
        let anchor_end = times[fp].unwrap().0;
        let step = anchor_end / fp as f64;
        for i in 0..fp {
            times[i] = Some((i as f64 * step, (i + 1) as f64 * step, 0.1));
        }
    }

    // Fill suffix.
    if lp < n - 1 {
        let anchor_start = times[lp].unwrap().1;
        let remaining = n - 1 - lp;
        let step = (total_dur - anchor_start) / remaining as f64;
        for (k, i) in (lp + 1..n).enumerate() {
            let s = anchor_start + k as f64 * step;
            times[i] = Some((s, s + step, 0.1));
        }
    }

    // Fill interior gaps.
    let mut i = 0;
    while i < n {
        if times[i].is_none() {
            let prev_end = times[..i]
                .iter()
                .rev()
                .find_map(|t| t.map(|(_, e, _)| e))
                .unwrap_or(0.0);
            let next_start = times[i + 1..]
                .iter()
                .find_map(|t| t.map(|(s, _, _)| s))
                .unwrap_or(total_dur);
            let gap_words = times[i..].iter().take_while(|t| t.is_none()).count();
            let step = (next_start - prev_end) / (gap_words + 1) as f64;
            for k in 0..gap_words {
                let s = prev_end + (k + 1) as f64 * step;
                times[i + k] = Some((s, s + step * 0.9, 0.1));
            }
            i += gap_words;
        } else {
            i += 1;
        }
    }
}

fn enforce_monotonicity(times: &mut [Option<(f64, f64, f32)>]) {
    let mut cursor = 0.0f64;
    for t in times.iter_mut() {
        if let Some((s, e, _)) = t.as_mut() {
            if *s < cursor {
                let dur = (*e - *s).max(0.02);
                *s = cursor;
                *e = cursor + dur;
            }
            cursor = *s;
        }
    }
}

fn build_empty_output(words: &[String]) -> Vec<AlignedWord> {
    words
        .iter()
        .map(|w| AlignedWord {
            word: w.clone(),
            normalized: normalize_word(w).join(" "),
            start: 0.0,
            end: 0.0,
            confidence: 0.0,
        })
        .collect()
}

fn platform_cache_dir() -> std::path::PathBuf {
    dirs_next::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from(".cache"))
        .join("lyrics-aligner")
        .join("models")
}
