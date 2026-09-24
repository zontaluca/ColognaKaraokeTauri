use std::path::{Path, PathBuf};
use std::sync::Arc;

use aligner_pipeline::AudioBuffer;
use aligner_wav2vec2::Wav2vecAligner;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::Value;
use tauri::AppHandle;

use crate::word_timing::{
    build_entries, estimate_lrc_offset_ms, generate_lrc, generate_lrc_from_json, line_windows,
    mean_confidence, parse_lyrics, split_by_counts, LyricsInput, TimedWord, WordEntry,
};

/// Result of [`run_alignment`] (words.json itself is written to the song dir).
pub struct AlignmentOutput {
    /// Synced LRC built from the word timings when the input lyrics were plain
    /// (unsynced) text; to be stored as the song's lyrics.
    pub generated_lrc: Option<String>,
}

/// Detect language of LRC text by counting stopword hits. Returns ISO-639-1 code
/// or `None` if no strong signal.
fn detect_lrc_language(lrc: &str) -> Option<&'static str> {
    let stopwords: &[(&str, &[&str])] = &[
        ("it", &["il", "la", "di", "che", "è", "e", "un", "una", "non", "per", "lo", "le", "dei", "gli", "con", "mi", "ti", "ci", "si", "ma", "come", "sono", "ha", "mia", "suo", "sua", "però", "così", "più", "giorno", "mare", "nome"]),
        ("en", &["the", "and", "of", "to", "in", "is", "you", "that", "it", "was", "for", "on", "are", "with", "as", "at", "be", "this", "have", "from", "or", "but", "we", "they", "will", "my", "your", "his", "her"]),
        ("es", &["el", "la", "de", "que", "y", "en", "un", "una", "ser", "se", "no", "por", "con", "su", "para", "como", "está", "tiene", "es", "pero", "más", "todo", "mi"]),
        ("fr", &["le", "la", "les", "de", "un", "une", "et", "est", "en", "que", "dans", "pour", "sur", "pas", "il", "elle", "je", "tu", "nous", "vous", "mais", "qui", "où"]),
        ("pt", &["o", "a", "de", "que", "e", "do", "da", "em", "um", "uma", "para", "é", "com", "não", "os", "as", "se", "por", "mais", "mas"]),
        ("de", &["der", "die", "das", "und", "ist", "in", "zu", "ein", "eine", "nicht", "mit", "sich", "auf", "auch", "es", "an", "als", "bei", "ich", "du", "er", "sie"]),
    ];

    let normalized: String = lrc
        .chars()
        .map(|c| if c.is_alphabetic() || c.is_whitespace() { c.to_ascii_lowercase() } else { ' ' })
        .collect();
    let tokens: Vec<&str> = normalized.split_whitespace().collect();
    if tokens.len() < 5 {
        return None;
    }

    let mut best: Option<(&'static str, usize)> = None;
    for (lang, words) in stopwords {
        let set: std::collections::HashSet<&&str> = words.iter().collect();
        let hits = tokens.iter().filter(|t| set.contains(t)).count();
        if best.map_or(true, |(_, h)| hits > h) {
            best = Some((lang, hits));
        }
    }
    best.filter(|(_, h)| *h >= 3).map(|(l, _)| l)
}

fn write_words_json(dir: &Path, words: &[WordEntry]) -> Result<Value, String> {
    let arr: Vec<Value> = words.iter().map(|w| w.to_json()).collect();
    let path = dir.join("words.json");
    let s = serde_json::to_string_pretty(&arr).map_err(|e| e.to_string())?;
    std::fs::write(&path, s).map_err(|e| e.to_string())?;
    Ok(Value::Array(arr))
}

fn write_empty_words(dir: &Path) -> Result<Value, String> {
    let path = dir.join("words.json");
    std::fs::write(&path, "[]").map_err(|e| e.to_string())?;
    Ok(Value::Array(vec![]))
}

// ─── Algorithm: wav2vec2 CTC forced alignment (single-pass) ──────────────────

fn wav2vec2_local_dir(language: &str) -> PathBuf {
    aligner_wav2vec2::default_local_dir(language)
}

type SharedAligner = Arc<Mutex<Wav2vecAligner>>;

/// wav2vec2 aligner kept loaded between consecutive jobs, keyed by language.
/// Building the ONNX session for XLSR-53 (~1.2 GB) takes seconds, so the job
/// queue reuses it and calls [`release_model_cache`] once it drains.
static MODEL_CACHE: Lazy<Mutex<Option<(String, SharedAligner)>>> = Lazy::new(|| Mutex::new(None));

/// Drop the cached wav2vec2 session (frees its memory once no alignment still
/// holds a reference to it).
pub fn release_model_cache() {
    if MODEL_CACHE.lock().take().is_some() {
        eprintln!("[aligner/W2V] model cache released");
    }
}

async fn load_aligner(language: &str, local_dir: PathBuf) -> Result<SharedAligner, String> {
    use aligner_wav2vec2::{AlignmentConfig as W2VConfig, ModelSource};

    {
        let mut cache = MODEL_CACHE.lock();
        match cache.as_ref() {
            Some((lang, aligner)) if lang == language => return Ok(aligner.clone()),
            // A model for another language: drop it before loading the new one
            // so the two never sit in memory together.
            Some(_) => *cache = None,
            None => {}
        }
    }

    let config = W2VConfig {
        source: ModelSource::LocalDir(local_dir),
        language: language.to_string(),
        ..Default::default()
    };
    // Session construction (graph optimisation of a large model) is CPU-bound:
    // keep it off the async runtime.
    let handle = tokio::runtime::Handle::current();
    let aligner = tokio::task::spawn_blocking(move || handle.block_on(Wav2vecAligner::new(config)))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let shared: SharedAligner = Arc::new(Mutex::new(aligner));
    *MODEL_CACHE.lock() = Some((language.to_string(), shared.clone()));
    Ok(shared)
}

/// Resample decoded vocals to 16 kHz and crop them to the sung range.
/// Returns the cropped buffer plus the (onset, offset) of the range in seconds.
fn prepare_vocals_16k(vocals: &AudioBuffer) -> Result<(AudioBuffer, f64, f64), String> {
    const SR: u32 = 16_000;
    let mut samples = crate::audio::resample_to(&vocals.samples, vocals.sample_rate, SR)?;
    let (vocal_onset, vocal_offset) = aligner_pipeline::detect_vocal_range(&samples, SR);
    let sr = SR as f64;
    let s_idx = ((vocal_onset * sr) as usize).min(samples.len());
    let e_idx = ((vocal_offset * sr) as usize).min(samples.len());
    if e_idx > s_idx {
        // Crop in place rather than copying the kept range into a new buffer.
        samples.truncate(e_idx);
        samples.drain(..s_idx);
    }
    Ok((AudioBuffer { samples, sample_rate: SR }, vocal_onset, vocal_offset))
}

/// wav2vec2 CTC forced alignment of `input` against the decoded vocals.
///
/// One inference pass produces the emissions; the whole text is aligned once,
/// then (for synced LRC whose timing agrees with the audio) every line is
/// re-aligned inside its own LRC window and the better-scoring placement of
/// the two is kept. Returns per-line timed words plus the estimated LRC offset.
async fn try_wav2vec2_ctc_alignment(
    input: &LyricsInput,
    vocals: Option<Arc<AudioBuffer>>,
    on_progress: &(dyn Fn(usize, usize) + Send + Sync),
) -> Option<(Vec<Vec<TimedWord>>, Option<i64>)> {
    let Some(vocals) = vocals else {
        eprintln!("[aligner/W2V] missing vocals.mp3");
        return None;
    };

    // Count words the way the wav2vec2 tokenizer does: tokens with no letters
    // (numbers, "&", "…") are dropped from the aligned output, and counting
    // them here would shift every following word onto the wrong line.
    let line_word_counts: Vec<usize> = input
        .lines
        .iter()
        .map(|l| aligner_wav2vec2::text::count_alignable_words(&l.text))
        .collect();
    let total_words: usize = line_word_counts.iter().sum();
    if total_words == 0 {
        eprintln!("[aligner/W2V] no words in lyrics");
        return None;
    }

    let concat_lyrics = input.joined_text();
    let language = detect_lrc_language(&concat_lyrics).unwrap_or("it");
    on_progress(0, total_words);

    let local_dir = wav2vec2_local_dir(language);
    if !local_dir.join("model.onnx").exists() || !local_dir.join("vocab.json").exists() {
        eprintln!(
            "[aligner/W2V] model files missing at {} — run scripts/fetch-binaries.sh",
            local_dir.display()
        );
        return None;
    }

    let prepared = tokio::task::spawn_blocking(move || prepare_vocals_16k(&vocals)).await;
    let (vocals_16k, vocal_onset, vocal_offset) = match prepared {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            eprintln!("[aligner/W2V] resample failed: {}", e);
            return None;
        }
        Err(e) => {
            eprintln!("[aligner/W2V] resample task failed: {}", e);
            return None;
        }
    };
    eprintln!(
        "[aligner/W2V] vocal range {:.2}..{:.2}s, {} samples, lang={}",
        vocal_onset,
        vocal_offset,
        vocals_16k.samples.len(),
        language,
    );

    let aligner = match load_aligner(language, local_dir).await {
        Ok(a) => a,
        Err(e) => {
            eprintln!("[aligner/W2V] init failed: {}", e);
            return None;
        }
    };

    let lines = input.lines.clone();
    let synced = input.synced;
    // ONNX inference + CTC take tens of seconds of CPU: run them on a blocking thread.
    let aligned = tokio::task::spawn_blocking(move || -> Result<_, String> {
        let mut aligner = aligner.lock();
        let emissions = aligner
            .emissions(vocals_16k, vocal_onset)
            .map_err(|e| e.to_string())?;
        let global: Vec<TimedWord> = aligner
            .align_emissions(&emissions, &concat_lyrics, None)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|w| TimedWord { word: w.word, start: w.start, end: w.end, confidence: Some(w.confidence) })
            .collect();
        // Walk aligned words alongside LRC lines, attributing each output word to
        // the line it came from (LRC token order is preserved by `align`).
        let mut per_line = split_by_counts(global, &line_word_counts);

        let offset_ms = if synced { estimate_lrc_offset_ms(&lines, &per_line) } else { None };
        if let Some(offset_ms) = offset_ms {
            let mut refined = 0usize;
            for (i, window) in line_windows(&lines, offset_ms, emissions.end_sec()).into_iter().enumerate() {
                let Some(window) = window else { continue };
                if line_word_counts[i] == 0 {
                    continue;
                }
                let Ok(words) = aligner.align_emissions(&emissions, &lines[i].text, Some(window)) else {
                    continue;
                };
                if words.len() != line_word_counts[i] {
                    continue;
                }
                let candidate: Vec<TimedWord> = words
                    .into_iter()
                    .map(|w| TimedWord { word: w.word, start: w.start, end: w.end, confidence: Some(w.confidence) })
                    .collect();
                if mean_confidence(&candidate) > mean_confidence(&per_line[i]) {
                    per_line[i] = candidate;
                    refined += 1;
                }
            }
            eprintln!(
                "[aligner/W2V] LRC offset {} ms; {} of {} lines improved by per-line windows",
                offset_ms,
                refined,
                lines.len()
            );
        }
        Ok((per_line, offset_ms))
    })
    .await;
    let (per_line, offset_ms) = match aligned {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            eprintln!("[aligner/W2V] align failed: {}", e);
            return None;
        }
        Err(e) => {
            eprintln!("[aligner/W2V] align task failed: {}", e);
            return None;
        }
    };
    on_progress(total_words, total_words);
    Some((per_line, offset_ms))
}

// ─── Public entry point ──────────────────────────────────────────────────────

/// True when [`run_alignment`] will actually need the decoded vocals (no cached
/// words.json and some lyrics to align against).
pub fn alignment_needed(dir: &Path, lrc: Option<&str>) -> bool {
    !dir.join("words.json").exists() && lrc.and_then(parse_lyrics).is_some()
}

/// `vocals` is the decoded vocals.mp3 at its native rate (shared with the pitch
/// stage so the file is decoded only once); `None` when it is missing.
///
/// Synced LRC and plain lyrics are both aligned; for plain lyrics a synced LRC
/// is generated from the word timings.
pub async fn run_alignment(
    dir: &Path,
    lrc: Option<&str>,
    vocals: Option<Arc<AudioBuffer>>,
    on_phrase_progress: &(dyn Fn(usize, usize) + Send + Sync),
) -> Result<AlignmentOutput, String> {
    let input = lrc.and_then(parse_lyrics);

    let words_path = dir.join("words.json");
    if words_path.exists() {
        let s = std::fs::read_to_string(&words_path).map_err(|e| e.to_string())?;
        let words: Value = serde_json::from_str(&s).map_err(|e| e.to_string())?;
        // Cached words of plain lyrics: rebuild the LRC they were timed against.
        let generated_lrc = input
            .as_ref()
            .filter(|i| !i.synced)
            .and_then(|i| generate_lrc_from_json(&i.lines, &words));
        return Ok(AlignmentOutput { generated_lrc });
    }

    let Some(input) = input else {
        eprintln!("[aligner] no lyrics — writing empty words.json");
        write_empty_words(dir)?;
        return Ok(AlignmentOutput { generated_lrc: None });
    };

    match try_wav2vec2_ctc_alignment(&input, vocals, on_phrase_progress).await {
        Some((per_line, offset_ms)) => {
            let entries = build_entries(&input.lines, per_line, offset_ms);
            let estimated = entries.iter().filter(|e| e.estimated).count();
            eprintln!(
                "[aligner/W2V] aligned {} words across {} lines ({} re-timed from low confidence)",
                entries.len(),
                input.lines.len(),
                estimated
            );
            let generated_lrc = (!input.synced && !entries.is_empty())
                .then(|| generate_lrc(&input.lines, &entries));
            write_words_json(dir, &entries)?;
            Ok(AlignmentOutput { generated_lrc })
        }
        None => {
            eprintln!("[aligner/W2V] failed; writing empty words.json");
            write_empty_words(dir)?;
            Ok(AlignmentOutput { generated_lrc: None })
        }
    }
}

// ─── Tauri commands ──────────────────────────────────────────────────────────

#[tauri::command(async)]
pub fn get_words(dir: String) -> Result<Value, String> {
    let words_path = PathBuf::from(&dir).join("words.json");
    if words_path.exists() {
        let s = std::fs::read_to_string(&words_path).map_err(|e| e.to_string())?;
        serde_json::from_str(&s).map_err(|e| e.to_string())
    } else {
        Ok(Value::Null)
    }
}

#[tauri::command]
pub fn get_cookies_file(app: AppHandle) -> Option<String> {
    crate::settings::load_settings(&app).cookies_file
}

#[tauri::command]
pub fn set_cookies_file(app: AppHandle, path: Option<String>) -> Result<(), String> {
    let mut settings = crate::settings::load_settings(&app);
    settings.cookies_file = path;
    crate::settings::save_settings(&app, &settings)
}

#[tauri::command]
pub fn get_cookie_browser(app: AppHandle) -> String {
    match crate::settings::load_settings(&app).cookie_browser {
        crate::settings::CookieBrowser::None => "none".into(),
        crate::settings::CookieBrowser::Safari => "safari".into(),
        crate::settings::CookieBrowser::Chrome => "chrome".into(),
        crate::settings::CookieBrowser::Firefox => "firefox".into(),
        crate::settings::CookieBrowser::Chromium => "chromium".into(),
    }
}

#[tauri::command]
pub fn set_cookie_browser(app: AppHandle, browser: String) -> Result<(), String> {
    let mut settings = crate::settings::load_settings(&app);
    settings.cookie_browser = match browser.as_str() {
        "safari" => crate::settings::CookieBrowser::Safari,
        "chrome" => crate::settings::CookieBrowser::Chrome,
        "firefox" => crate::settings::CookieBrowser::Firefox,
        "chromium" => crate::settings::CookieBrowser::Chromium,
        _ => crate::settings::CookieBrowser::None,
    };
    crate::settings::save_settings(&app, &settings)
}

#[tauri::command]
pub fn get_alignment_mode(app: AppHandle) -> String {
    crate::settings::load_settings(&app).alignment_mode.as_str().into()
}

#[tauri::command]
pub fn set_alignment_mode(app: AppHandle, mode: String) -> Result<(), String> {
    let parsed = crate::settings::AlignmentMode::from_str(&mode)
        .ok_or_else(|| format!("unknown alignment mode: {}", mode))?;
    let mut settings = crate::settings::load_settings(&app);
    settings.alignment_mode = parsed;
    crate::settings::save_settings(&app, &settings)
}
