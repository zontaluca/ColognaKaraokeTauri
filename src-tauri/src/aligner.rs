use std::path::{Path, PathBuf};

use aligner_pipeline::AudioBuffer;
use serde_json::{json, Value};
use tauri::AppHandle;

#[derive(Debug, Clone)]
struct WordEntry {
    word: String,
    start_ms: u64,
    end_ms: u64,
    line: Option<usize>,
}

impl WordEntry {
    fn to_json(&self) -> Value {
        let mut obj = serde_json::Map::new();
        obj.insert("word".into(), Value::String(self.word.clone()));
        obj.insert("start_ms".into(), json!(self.start_ms));
        obj.insert("end_ms".into(), json!(self.end_ms));
        if let Some(l) = self.line {
            obj.insert("line".into(), json!(l));
        }
        Value::Object(obj)
    }
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

fn clean_token(raw: &str) -> String {
    raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '-')
        .to_string()
}

fn load_wav_mono_16k(path: &Path) -> Result<AudioBuffer, String> {
    let samples = crate::audio::load_wav_mono_16k(path)?;
    Ok(AudioBuffer { samples, sample_rate: 16_000 })
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

async fn try_wav2vec2_ctc_alignment(
    dir: &Path,
    lrc_text: &str,
    on_progress: &(dyn Fn(usize, usize) + Send + Sync),
) -> Option<Vec<WordEntry>> {
    use aligner_wav2vec2::{AlignmentConfig as W2VConfig, ModelSource, Wav2vecAligner};

    let vocals_path = dir.join("vocals.mp3");
    if !vocals_path.exists() {
        eprintln!("[aligner/W2V] missing vocals.mp3");
        return None;
    }
    let mut vocals = match load_wav_mono_16k(&vocals_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[aligner/W2V] WAV load failed: {}", e);
            return None;
        }
    };

    let lrc_lines = crate::lyrics::parse_lrc(lrc_text);
    if lrc_lines.is_empty() {
        eprintln!("[aligner/W2V] empty LRC");
        return None;
    }
    let total_words: usize = lrc_lines
        .iter()
        .map(|l| l.text.split_whitespace().count())
        .sum();
    if total_words == 0 {
        eprintln!("[aligner/W2V] no words in LRC");
        return None;
    }

    let language = detect_lrc_language(lrc_text).unwrap_or("it");
    on_progress(0, total_words);

    let (vocal_onset, vocal_offset) = aligner_pipeline::detect_vocal_range(
        &vocals.samples,
        vocals.sample_rate,
    );
    let sr = vocals.sample_rate as f64;
    let s_idx = ((vocal_onset * sr) as usize).min(vocals.samples.len());
    let e_idx = ((vocal_offset * sr) as usize).min(vocals.samples.len());
    if e_idx > s_idx {
        vocals.samples = vocals.samples[s_idx..e_idx].to_vec();
    }
    eprintln!(
        "[aligner/W2V] vocal range {:.2}..{:.2}s, {} samples, lang={}",
        vocal_onset,
        vocal_offset,
        vocals.samples.len(),
        language,
    );

    let local_dir = wav2vec2_local_dir(language);
    if !local_dir.join("model.onnx").exists() || !local_dir.join("vocab.json").exists() {
        eprintln!(
            "[aligner/W2V] model files missing at {} — run scripts/fetch-binaries.sh",
            local_dir.display()
        );
        return None;
    }
    let config = W2VConfig {
        source: ModelSource::LocalDir(local_dir),
        language: language.to_string(),
        ..Default::default()
    };
    let mut aligner = match Wav2vecAligner::new(config).await {
        Ok(a) => a,
        Err(e) => {
            eprintln!("[aligner/W2V] init failed: {}", e);
            return None;
        }
    };

    let concat_lyrics: String = lrc_lines
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<&str>>()
        .join(" ");

    let aligned = match aligner.align(&vocals, &concat_lyrics, vocal_onset) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("[aligner/W2V] align failed: {}", e);
            return None;
        }
    };
    on_progress(total_words, total_words);

    // Walk aligned words alongside LRC lines, attributing each output word to
    // the line it came from (LRC token order is preserved by `align`).
    let mut cursor = 0usize;
    let mut out: Vec<WordEntry> = Vec::with_capacity(aligned.len());
    for (line_idx, line) in lrc_lines.iter().enumerate() {
        let line_word_count = line.text.split_whitespace().count();
        let line_end_ms = lrc_lines
            .get(line_idx + 1)
            .map(|l| l.ts_ms)
            .unwrap_or(u64::MAX);
        for _ in 0..line_word_count {
            if cursor >= aligned.len() {
                break;
            }
            let w = &aligned[cursor];
            cursor += 1;
            let clean = clean_token(&w.word);
            if clean.is_empty() {
                continue;
            }
            let start_ms = ((w.start * 1000.0).max(0.0)).round() as u64;
            let mut end_ms = ((w.end * 1000.0).max(0.0)).round() as u64;
            if end_ms <= start_ms {
                end_ms = start_ms + 50;
            }
            // Clamp into LRC neighbour window so a single misaligned word can't
            // pull the next line's highlight forward.
            let start_ms = start_ms.min(line_end_ms.saturating_sub(50));
            let end_ms = end_ms.min(line_end_ms);
            out.push(WordEntry {
                word: clean,
                start_ms,
                end_ms,
                line: Some(line_idx),
            });
        }
    }

    // Global monotonicity guard.
    let mut last = 0_u64;
    for w in out.iter_mut() {
        if w.start_ms < last {
            w.start_ms = last;
        }
        if w.end_ms <= w.start_ms {
            w.end_ms = w.start_ms + 50;
        }
        last = w.start_ms;
    }

    if out.is_empty() {
        None
    } else {
        eprintln!(
            "[aligner/W2V] aligned {} words across {} lines",
            out.len(),
            lrc_lines.len()
        );
        Some(out)
    }
}

// ─── Public entry point ──────────────────────────────────────────────────────

pub async fn run_alignment(
    dir: &Path,
    lrc: Option<&str>,
    on_phrase_progress: &(dyn Fn(usize, usize) + Send + Sync),
) -> Result<Value, String> {
    let words_path = dir.join("words.json");
    if words_path.exists() {
        let s = std::fs::read_to_string(&words_path).map_err(|e| e.to_string())?;
        return serde_json::from_str(&s).map_err(|e| e.to_string());
    }

    let lrc_text = match lrc.filter(|t| t.contains('[')) {
        Some(t) => t,
        None => {
            eprintln!("[aligner] no synced LRC — writing empty words.json");
            return write_empty_words(dir);
        }
    };

    match try_wav2vec2_ctc_alignment(dir, lrc_text, on_phrase_progress).await {
        Some(words) => write_words_json(dir, &words),
        None => {
            eprintln!("[aligner/W2V] failed; writing empty words.json");
            write_empty_words(dir)
        }
    }
}

// ─── Tauri commands ──────────────────────────────────────────────────────────

#[tauri::command]
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
