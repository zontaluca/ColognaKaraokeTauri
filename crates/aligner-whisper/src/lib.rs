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

/// LRC-derived phrase anchor: a known phrase start time + the words in that
/// phrase.  When these are available the aligner partitions the audio per
/// phrase and runs DTW on each slice independently, eliminating the
/// long-song proportional-token-assignment drift.
#[derive(Debug, Clone)]
pub struct PhraseAnchor {
    pub time_sec: f64,
    pub words: Vec<String>,
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

        let (orig_words, norm_words, norm_map) = normalize_lyrics(lyrics);
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

        // norm_map[orig_idx] = (start_norm, end_norm); build norm_word_idx → orig_word_idx.
        // Needed because contractions expand (e.g. "don't" → ["do","not"]), making
        // norm_words.len() > orig_words.len(). token_to_word must stay within orig bounds.
        let mut norm_to_orig: Vec<usize> = vec![0; norm_words.len()];
        for (orig_idx, &(s, e)) in norm_map.iter().enumerate() {
            for ni in s..e {
                norm_to_orig[ni] = orig_idx;
            }
        }

        // Build token → orig word index mapping.
        let token_to_word: Vec<usize> = word_token_ids
            .iter()
            .enumerate()
            .flat_map(|(norm_wi, toks)| std::iter::repeat(norm_to_orig[norm_wi]).take(toks.len()))
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

        // Detect vocal onset/offset using frame-level energy.  Songs often have
        // 10–60 s of instrumental intro/outro; if we use total_dur for the
        // proportional token-to-chunk split, late words get assigned to empty
        // post-vocal chunks and drift massively.  Use active_dur (onset→offset)
        // instead so token density matches vocal density.
        let (vocal_onset, vocal_offset) = detect_vocal_range(&vocals.samples, vocals.sample_rate);
        let active_dur = (vocal_offset - vocal_onset).max(1.0);

        let chunks = make_chunks(
            &vocals.samples,
            self.config.chunk_seconds,
            self.config.overlap_seconds,
        );
        info!(
            "aligning {} words ({} tokens) across {} chunks, total {:.1}s, active {:.1}s ({:.1}s→{:.1}s)",
            orig_words.len(),
            flat_tokens.len(),
            chunks.len(),
            total_dur,
            active_dur,
            vocal_onset,
            vocal_offset,
        );

        // word_times[i] = Some((start, end, confidence)) or None.
        let mut word_times: Vec<Option<(f64, f64, f32)>> =
            vec![None; orig_words.len()];

        for (chunk_samples, chunk_offset) in &chunks {
            let chunk_end =
                (chunk_offset + self.config.chunk_seconds as f64).min(total_dur);

            // Skip chunks entirely outside the active vocal range.
            if *chunk_offset >= vocal_offset || chunk_end <= vocal_onset {
                continue;
            }

            // Map chunk boundaries into the active-vocal timeline, then compute
            // proportional token span against active_dur.
            let active_chunk_start = (chunk_offset - vocal_onset).max(0.0);
            let active_chunk_end = (chunk_end - vocal_onset).min(active_dur);
            let frac_s = (active_chunk_start / active_dur).clamp(0.0, 1.0);
            let frac_e = (active_chunk_end / active_dur).clamp(0.0, 1.0);
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
                // The first content token in every chunk always gets fs=0 because
                // the DTW path is forced to start at (0,0).  This bloats span_dur
                // with all the pre-speech frames and causes a massive back-shift.
                // Use `fe` (real DTW end of the span) as the effective start: span
                // collapses to 1 frame, back-shift becomes ~13 ms, and t_start ≈
                // fe*20 ms which closely tracks the true onset.
                let effective_fs = if local_idx == 0 { fe } else { fs };
                let span_dur = (fe + 1).saturating_sub(effective_fs) as f64 * FRAME_MS / 1000.0;
                // Cross-attention peaks slightly past mid-word for Whisper Small.
                // 0.65 × span_dur back-shift was empirically optimal on the Italian
                // TTS fixture (attention lags word onset by ~65% of the span).
                let raw_start = chunk_offset + effective_fs as f64 * FRAME_MS / 1000.0;
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

        // Correct leading-word placement errors caused by instrumental intros:
        // if the first N words land far before the next word (large forward jump),
        // back-extrapolate their positions from the first reliable anchor.
        fix_leading_outliers(&mut word_times);

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

    /// Align lyrics to audio using LRC phrase anchors.
    ///
    /// For each phrase `i`, extract the audio window
    /// `[phrases[i].time_sec, phrases[i+1].time_sec]` (plus a small DTW
    /// context pad past the window) and run the regular
    /// [`ForcedAligner::align`] on just the words of that phrase.  Output
    /// word times are then clamped to the strict phrase window so no word
    /// bleeds into the next phrase's LRC range.
    ///
    /// `dtw_pad_sec` extends the audio fed to DTW past the phrase boundary
    /// so the decoder has room for the final word's attention peak; the
    /// extra frames are dropped when clamping back to the phrase window.
    pub fn align_with_phrases(
        &self,
        vocals: &AudioBuffer,
        phrases: &[PhraseAnchor],
        dtw_pad_sec: f64,
    ) -> Result<Vec<AlignedWord>, AlignError> {
        if vocals.sample_rate != 16_000 {
            return Err(AlignError::BadSampleRate(vocals.sample_rate));
        }
        let total_dur = vocals.samples.len() as f64 / vocals.sample_rate as f64;
        let sr = vocals.sample_rate as f64;
        let mut out: Vec<AlignedWord> = Vec::new();

        for (i, phrase) in phrases.iter().enumerate() {
            if phrase.words.is_empty() {
                continue;
            }
            let start = phrase.time_sec.max(0.0).min(total_dur);
            // Strict phrase window end — where the NEXT phrase begins (or song end).
            let phrase_end = phrases
                .get(i + 1)
                .map(|p| p.time_sec.min(total_dur))
                .unwrap_or(total_dur);
            // Extended audio buffer end for DTW context.
            let sub_end = (phrase_end + dtw_pad_sec).min(total_dur);
            if phrase_end <= start + 0.05 {
                // degenerate window — place words at the phrase start with zero dur
                for w in &phrase.words {
                    out.push(AlignedWord {
                        word: w.clone(),
                        normalized: normalize_word(w).join(" "),
                        start,
                        end: start + 0.05,
                        confidence: 0.1,
                    });
                }
                continue;
            }

            let s_idx = (start * sr) as usize;
            let e_idx = ((sub_end * sr) as usize).min(vocals.samples.len());
            if e_idx <= s_idx {
                continue;
            }

            // Compute two vocal-end estimates over the sub-buffer:
            //   • last_vocal — last frame above the RMS threshold.
            //   • first_seg_end — end of the FIRST contiguous vocal block,
            //     cut by the first ≥ 800 ms silence.
            //
            // Decision: if the first segment alone is long enough to host
            // the phrase's word count at a reasonable pace (≥ 300 ms/word),
            // trust first_seg_end — this trims an instrumental tail that
            // follows the lyrics (e.g. song3 line 19 + 14 s guitar solo).
            // Otherwise the phrase lyrics span multiple vocal segments
            // (e.g. song4 "Yeah … let's go … ha-ha-ha" with 1 s gaps),
            // and we must use last_vocal so they can all be placed.
            let (_sub_onset, last_vocal_local) =
                detect_vocal_range(&vocals.samples[s_idx..e_idx], vocals.sample_rate);
            let first_seg_end_local = find_first_vocal_segment_end(
                &vocals.samples[s_idx..e_idx],
                vocals.sample_rate,
            );
            let min_required_dur = phrase.words.len() as f64 * 0.30;
            let vocal_end_local = if first_seg_end_local >= min_required_dur {
                first_seg_end_local
            } else {
                last_vocal_local
            };
            let vocal_end_abs = (start + vocal_end_local + 0.2).min(sub_end);
            let effective_end = vocal_end_abs.max(start + 0.5);
            let new_e_idx = ((effective_end * sr) as usize).min(vocals.samples.len());

            let sub = AudioBuffer {
                samples: vocals.samples[s_idx..new_e_idx].to_vec(),
                sample_rate: vocals.sample_rate,
            };
            let lyrics = phrase.words.join(" ");
            let aligned = self.align(&sub, &lyrics)?;
            // Output window follows detected vocal segment, not raw LRC marker.
            // Global monotonicity cursor below prevents overlap with the next
            // phrase even when this phrase extends past its LRC end.
            let strict_end = effective_end;
            let window = (strict_end - start).max(0.05);

            for mut w in aligned {
                let s_local = w.start.clamp(0.0, window - 0.02);
                let e_local = w.end.clamp(s_local + 0.02, window);
                w.start = start + s_local;
                w.end = start + e_local;
                out.push(w);
            }
        }

        // Enforce global monotonicity in case a per-phrase end overlaps the next
        // phrase's earliest word (rare, but possible with aggressive back-shift).
        let mut cursor = 0.0_f64;
        for w in out.iter_mut() {
            if w.start < cursor {
                w.start = cursor;
            }
            if w.end < w.start + 0.02 {
                w.end = w.start + 0.02;
            }
            cursor = w.start;
        }

        Ok(out)
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

/// Detect the first and last time (seconds) where vocal energy exceeds a
/// relative threshold.  Operates on the separated vocals track, so the only
/// energy present is from singing.  Used to crop the proportional token-to-
/// chunk split to the actual vocal range and avoid placing late words in
/// post-vocal silence.
fn detect_vocal_range(samples: &[f32], sample_rate: u32) -> (f64, f64) {
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

    // Threshold relative to 95th-percentile RMS (robust to impulse peaks).
    // Vocal-separation residuals sit at roughly 10–15% of typical vocal RMS,
    // so 15% of p95 cleanly excludes them while keeping all real singing.
    let mut sorted = rms.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p95 = sorted[((sorted.len() as f64 * 0.95) as usize).min(sorted.len() - 1)];
    let thr = p95 * 0.15;

    // Smooth: require a majority of a 5-window (500 ms) neighborhood to exceed
    // threshold — long enough to reject isolated bleed spikes, short enough to
    // catch quiet onset syllables.
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
    // Add one window past last active so the final word isn't cut off.
    let offset = ((last + 1) as f64 * win_sec).min(samples.len() as f64 / sample_rate as f64);
    (onset, offset)
}

/// Find the end (in seconds) of the FIRST contiguous vocal segment in
/// `samples`.  A "gap" is ≥ 800 ms of consecutive 100 ms windows whose RMS
/// falls below 15 % of the p95 RMS — same threshold as `detect_vocal_range`.
///
/// Used by phrase-level alignment to:
///   • Shrink the output window when an instrumental tail follows the vocal.
///   • Extend the output window past an LRC marker that is set too early.
///
/// Returns the buffer duration when no gap is found (vocal continuous to
/// the end of the sub-buffer).
fn find_first_vocal_segment_end(samples: &[f32], sample_rate: u32) -> f64 {
    let win_samples = (sample_rate as usize / 10).max(1); // 100 ms
    let rms: Vec<f32> = samples
        .chunks(win_samples)
        .map(|w| {
            let s: f32 = w.iter().map(|x| x * x).sum();
            (s / w.len() as f32).sqrt()
        })
        .collect();
    let total_dur = samples.len() as f64 / sample_rate as f64;
    if rms.is_empty() {
        return total_dur;
    }

    let mut sorted = rms.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p95 = sorted[((sorted.len() as f64 * 0.95) as usize).min(sorted.len() - 1)];
    let thr = p95 * 0.15;

    let win_sec = win_samples as f64 / sample_rate as f64;
    // 800 ms minimum gap to flag as instrumental break.  Inter-word gaps in
    // pop vocals are typically < 500 ms (e.g. 241 ms between "Ey," and
    // "Tití" — 3 windows below threshold), so a 300 ms threshold produces
    // false positives mid-phrase.  Real solo / interlude gaps are ≥ 1 s.
    const GAP_WINDOWS: usize = 8; // 800 ms

    // Skip leading silence to locate vocal onset.
    let n = rms.len();
    let mut i = 0;
    while i < n && rms[i] <= thr {
        i += 1;
    }
    if i >= n {
        return total_dur;
    }

    // Walk through vocal block; declare end at first ≥ GAP_WINDOWS run of
    // sub-threshold windows.
    while i < n {
        if rms[i] <= thr {
            let mut k = i;
            while k < n && rms[k] <= thr {
                k += 1;
            }
            if k - i >= GAP_WINDOWS {
                return (i as f64 * win_sec).min(total_dur);
            }
            i = k;
        } else {
            i += 1;
        }
    }
    total_dur
}

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
    // When multiple tokens collide (same back-shifted t_start), distribute
    // them evenly between the last distinct anchor and the next non-colliding
    // word. The step is capped at 200 ms so a long inter-phrase pause doesn't
    // spread colliding words across the entire silence.
    const MAX_STEP_S: f64 = 0.200;
    let n = times.len();
    let mut cursor = 0.0_f64;
    let mut i = 0;
    while i < n {
        match times[i] {
            None => { i += 1; }
            Some((s, _, _)) if s > cursor => {
                cursor = s;
                i += 1;
            }
            Some(_) => {
                // Collect collision chain.
                let chain_start = i;
                let mut j = i;
                while j < n {
                    match times[j] {
                        Some((sj, _, _)) if sj <= cursor => j += 1,
                        None => j += 1,
                        _ => break,
                    }
                }
                let some_in_chain: Vec<usize> =
                    (chain_start..j).filter(|&k| times[k].is_some()).collect();
                let m = some_in_chain.len();
                if m == 0 { i = j; continue; }

                let next_anchor = (j..n)
                    .find_map(|k| {
                        times[k].and_then(|(s, _, _)| (s > cursor).then_some(s))
                    })
                    .unwrap_or(cursor + m as f64 * MAX_STEP_S);

                // Cap step so inter-phrase silence doesn't swallow the chain.
                let natural_step = (next_anchor - cursor) / (m + 1) as f64;
                let step = natural_step.min(MAX_STEP_S);
                for (rank, &k) in some_in_chain.iter().enumerate() {
                    if let Some((s, e, c)) = times[k] {
                        let new_s = cursor + (rank + 1) as f64 * step;
                        let dur = (e - s).max(0.02);
                        times[k] = Some((new_s, (new_s + dur).min(next_anchor), c));
                    }
                }
                cursor += m as f64 * step;
                i = j;
            }
        }
    }
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

/// When a song has a long instrumental intro the DTW (0,0) start constraint
/// anchors the first few lyrics words in the intro music instead of at the
/// real vocal onset.  This shows up as a large forward jump between word k
/// and word k+1 (e.g. words 0-1 at 1.5 s, word 2 at 15 s).
///
/// Detect that pattern and back-extrapolate the outlier run from the first
/// trustworthy anchor, using the local average gap of the following words as
/// the per-word step.
fn fix_leading_outliers(times: &mut [Option<(f64, f64, f32)>]) {
    const JUMP_THRESHOLD_S: f64 = 3.0;
    const LOOK_AHEAD: usize = 5;

    let n = times.len();

    // Find the first large forward jump within the first 10 words.
    let search_end = n.saturating_sub(1).min(10);
    for jump_at in 0..search_end {
        let Some((s_before, _, _)) = times[jump_at] else { continue };
        let Some((s_after, _, _)) = times[jump_at + 1] else { continue };

        if s_after - s_before < JUMP_THRESHOLD_S {
            continue;
        }

        // Identify the contiguous outlier run ending at jump_at (all close together).
        let mut run_start = jump_at;
        while run_start > 0 {
            match (times[run_start - 1], times[run_start]) {
                (Some((sp, _, _)), Some((sc, _, _))) if sc - sp < JUMP_THRESHOLD_S => {
                    run_start -= 1;
                }
                _ => break,
            }
        }

        // Guard: only treat as outlier if the entire run is tightly clustered
        // relative to the jump.  A natural inter-phrase pause (song2: words 0-7
        // span 1.7 s then jump 3.8 s to next phrase) has a large cluster/jump
        // ratio and should NOT be corrected.  A real intro-drag outlier (song3:
        // words 0-1 span 0.2 s then jump 13.5 s) has a tiny ratio.
        let run_span = match (times[run_start], times[jump_at]) {
            (Some((a, _, _)), Some((b, _, _))) => (b - a).abs(),
            _ => 0.0,
        };
        let jump_size = s_after - s_before;
        if run_span / jump_size > 0.20 {
            // Cluster is too spread out — this looks like a real phrase gap, not
            // a DTW start-drag outlier.
            continue;
        }

        // Estimate per-word step from the words immediately after the jump.
        let end = (jump_at + 1 + LOOK_AHEAD).min(n);
        let after: Vec<f64> = times[jump_at + 1..end]
            .iter()
            .filter_map(|t| t.map(|(s, _, _)| s))
            .collect();
        let local_gap = if after.len() >= 2 {
            (after.last().unwrap() - after[0]) / (after.len() - 1) as f64
        } else {
            0.200
        };

        // Back-extrapolate: word run_end is 1 step before s_after, etc.
        for (rank, idx) in (run_start..=jump_at).rev().enumerate() {
            let offset = (rank + 1) as f64;
            let new_s = (s_after - offset * local_gap).max(0.0);
            if let Some((old_s, old_e, c)) = times[idx] {
                let dur = (old_e - old_s).max(0.02);
                times[idx] = Some((new_s, new_s + dur, c));
            }
        }

        // Only fix the first large jump.
        break;
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
