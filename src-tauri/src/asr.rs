//! Speech recognition with NVIDIA Parakeet TDT 0.6B v3 (ONNX Runtime, through
//! `parakeet-rs`). One multilingual model (25 European languages) with native
//! word timestamps, used where wav2vec2 forced alignment can't run:
//! - songs without any lyrics are transcribed into timed words;
//! - lyrics in a language with no wav2vec2 model installed are timed by
//!   matching them against the transcription (see `word_timing`).
//!
//! Model files live in `<cache>/cologna-karaoke/parakeet-tdt-0.6b-v3/`
//! (fetched by `scripts/fetch-binaries.sh`, INT8 or FP32 export).

use std::path::PathBuf;
use std::sync::Arc;

use aligner_pipeline::AudioBuffer;
use once_cell::sync::Lazy;
use parakeet_rs::{ParakeetTDT, TimedToken, TimestampMode, Transcriber};
use parking_lot::Mutex;

use crate::word_timing::TimedWord;

const SAMPLE_RATE: u32 = 16_000;
/// TDT models handle a few minutes per pass; songs are split into chunks of
/// at most this length, cut at the quietest point near the end of each chunk.
const MAX_CHUNK_SEC: f64 = 90.0;
/// How far back from a chunk's maximum end to look for a quiet cut point.
const CUT_SEARCH_SEC: f64 = 15.0;
/// RMS window used to find the quietest cut point.
const CUT_WINDOW_SEC: f64 = 0.25;

pub fn model_dir() -> PathBuf {
    dirs_next::cache_dir()
        .unwrap_or_else(|| PathBuf::from(".cache"))
        .join("cologna-karaoke")
        .join("parakeet-tdt-0.6b-v3")
}

/// True when the Parakeet model files are installed (INT8 or FP32 export).
pub fn is_available() -> bool {
    let dir = model_dir();
    let any = |names: &[&str]| names.iter().any(|n| dir.join(n).exists());
    dir.join("vocab.txt").exists()
        && any(&["encoder-model.onnx", "encoder-model.int8.onnx"])
        && any(&["decoder_joint-model.onnx", "decoder_joint-model.int8.onnx"])
}

type SharedModel = Arc<Mutex<ParakeetTDT>>;

/// Loaded model kept between consecutive queued jobs, like the wav2vec2 one;
/// freed by [`release_model_cache`] when the queue drains.
static MODEL_CACHE: Lazy<Mutex<Option<SharedModel>>> = Lazy::new(|| Mutex::new(None));

pub fn release_model_cache() {
    if MODEL_CACHE.lock().take().is_some() {
        eprintln!("[asr] Parakeet model cache released");
    }
}

async fn load_model() -> Result<SharedModel, String> {
    if let Some(model) = MODEL_CACHE.lock().as_ref() {
        return Ok(model.clone());
    }
    let dir = model_dir();
    // Building the ONNX sessions is CPU-bound: keep it off the async runtime.
    let model = tokio::task::spawn_blocking(move || ParakeetTDT::from_pretrained(&dir, None))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("Parakeet load failed: {}", e))?;
    let shared: SharedModel = Arc::new(Mutex::new(model));
    *MODEL_CACHE.lock() = Some(shared.clone());
    Ok(shared)
}

/// Transcribe decoded vocals (any rate, mono) into timed words on the song
/// timeline.
pub async fn transcribe(vocals: Arc<AudioBuffer>) -> Result<Vec<TimedWord>, String> {
    if !is_available() {
        return Err(format!("Parakeet model not found at {}", model_dir().display()));
    }
    let model = load_model().await?;
    tokio::task::spawn_blocking(move || -> Result<Vec<TimedWord>, String> {
        let samples = crate::audio::resample_to(&vocals.samples, vocals.sample_rate, SAMPLE_RATE)?;
        // Skip the instrumental intro/outro: less audio to decode and fewer
        // hallucinated words on silence.
        let (onset, offset) = aligner_pipeline::detect_vocal_range(&samples, SAMPLE_RATE);
        let s_idx = ((onset * SAMPLE_RATE as f64) as usize).min(samples.len());
        let e_idx = ((offset * SAMPLE_RATE as f64) as usize).min(samples.len());
        let (base_sec, voiced) = if e_idx > s_idx {
            (onset, &samples[s_idx..e_idx])
        } else {
            (0.0, &samples[..])
        };

        let mut model = model.lock();
        let mut words = Vec::new();
        for (a, b) in chunk_bounds(voiced, SAMPLE_RATE) {
            let result = model
                .transcribe_samples(voiced[a..b].to_vec(), SAMPLE_RATE, 1, Some(TimestampMode::Words))
                .map_err(|e| format!("Parakeet transcription failed: {}", e))?;
            let chunk_start = base_sec + a as f64 / SAMPLE_RATE as f64;
            words.extend(tokens_to_words(&result.tokens, chunk_start));
        }
        eprintln!("[asr] Parakeet transcribed {} words", words.len());
        Ok(words)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Word tokens → timed words on the song timeline. Standalone punctuation
/// tokens are attached to the previous word (kept for line segmentation).
fn tokens_to_words(tokens: &[TimedToken], offset_sec: f64) -> Vec<TimedWord> {
    let mut out: Vec<TimedWord> = Vec::with_capacity(tokens.len());
    for t in tokens {
        let text = t.text.trim();
        if text.is_empty() {
            continue;
        }
        if !text.chars().any(char::is_alphanumeric) {
            if let Some(prev) = out.last_mut() {
                prev.word.push_str(text);
            }
            continue;
        }
        out.push(TimedWord {
            word: text.to_string(),
            start: offset_sec + t.start as f64,
            end: offset_sec + t.end as f64,
            confidence: None,
            estimated: false,
        });
    }
    out
}

/// Split `samples` into `[start, end)` ranges of at most `MAX_CHUNK_SEC`,
/// ending each chunk at the quietest window of its last `CUT_SEARCH_SEC`.
fn chunk_bounds(samples: &[f32], sample_rate: u32) -> Vec<(usize, usize)> {
    let sr = sample_rate as f64;
    let max_len = (MAX_CHUNK_SEC * sr) as usize;
    let search = (CUT_SEARCH_SEC * sr) as usize;
    let win = ((CUT_WINDOW_SEC * sr) as usize).max(1);

    let mut out = Vec::new();
    let mut pos = 0;
    while pos < samples.len() {
        let hard_end = pos + max_len;
        if hard_end >= samples.len() {
            out.push((pos, samples.len()));
            break;
        }
        let from = hard_end.saturating_sub(search).max(pos + win);
        let mut best = hard_end;
        let mut best_energy = f32::MAX;
        let mut w = from;
        while w + win <= hard_end {
            let energy: f32 = samples[w..w + win].iter().map(|s| s * s).sum();
            if energy < best_energy {
                best_energy = energy;
                best = w + win / 2;
            }
            w += win / 2;
        }
        out.push((pos, best));
        pos = best;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_cover_audio_and_cut_in_silence() {
        let sr = 1_000u32; // small rate keeps the test fast
        let mut samples = vec![0.5f32; 200_000]; // 200 s
        // Silence at 85-86 s, inside the search window of the first chunk.
        for s in &mut samples[85_000..86_000] {
            *s = 0.0;
        }
        let chunks = chunk_bounds(&samples, sr);
        assert_eq!(chunks.first().unwrap().0, 0);
        assert_eq!(chunks.last().unwrap().1, samples.len());
        assert!(chunks.windows(2).all(|w| w[0].1 == w[1].0));
        assert!(chunks.iter().all(|(a, b)| b - a <= 90_000));
        let cut = chunks[0].1;
        assert!((85_000..=86_000).contains(&cut), "cut at {}", cut);
    }

    #[test]
    fn punctuation_tokens_join_previous_word() {
        let tok = |text: &str, start: f32, end: f32| TimedToken { text: text.into(), start, end };
        let words = tokens_to_words(&[tok("Ciao", 0.0, 0.4), tok(",", 0.4, 0.4), tok("mondo", 0.5, 0.9), tok(".", 0.9, 0.9)], 10.0);
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].word, "Ciao,");
        assert_eq!(words[1].word, "mondo.");
        assert!((words[1].start - 10.5).abs() < 1e-6);
    }
}
