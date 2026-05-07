use std::path::{Path, PathBuf};

#[cfg(feature = "metal")]
use aligner_pipeline::{AlignedWord, AudioBuffer};
#[cfg(feature = "metal")]
use aligner_whisper::{AlignmentConfig, ForcedAligner, PhraseAnchor, WhisperModel};
#[cfg(feature = "metal")]
use serde_json::json;
use serde_json::Value;
use tauri::AppHandle;

#[cfg(feature = "metal")]
use crate::settings::AlignmentMode;

#[cfg(feature = "metal")]
#[derive(Debug, Clone)]
struct WordEntry {
    word: String,
    start_ms: u64,
    end_ms: u64,
    line: Option<usize>,
}

#[cfg(feature = "metal")]
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
#[cfg(feature = "metal")]
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

#[cfg(feature = "metal")]
fn vowel_count(word: &str) -> usize {
    word.chars()
        .filter(|c| matches!(c.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u' | 'y' | 'à' | 'è' | 'é' | 'ì' | 'ò' | 'ù'))
        .count()
        .max(1)
}

#[cfg(feature = "metal")]
fn clean_token(raw: &str) -> String {
    raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '-')
        .to_string()
}

#[cfg(feature = "metal")]
fn letters_only_lower(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Distribute words within a line time window by vowel count weight.
/// Used as last-resort fallback when alignment fails for a line.
#[cfg(feature = "metal")]
fn distribute_words_in_line(
    tokens: &[&str],
    line_idx: usize,
    start_ms: u64,
    end_ms: u64,
    out: &mut Vec<WordEntry>,
) {
    if tokens.is_empty() || end_ms <= start_ms {
        return;
    }
    let weights: Vec<f64> = tokens.iter().map(|w| vowel_count(w) as f64).collect();
    let total_w: f64 = weights.iter().sum::<f64>().max(1.0);
    let dur_ms = (end_ms - start_ms) as f64;

    let mut t = start_ms as f64;
    for (i, w) in tokens.iter().enumerate() {
        let word_dur = dur_ms * (weights[i] / total_w);
        let ws = t;
        let we = if i == tokens.len() - 1 { end_ms as f64 } else { t + word_dur };
        t = we;
        let clean = clean_token(w);
        if clean.is_empty() {
            continue;
        }
        out.push(WordEntry {
            word: clean,
            start_ms: ws.round() as u64,
            end_ms: we.round() as u64,
            line: Some(line_idx),
        });
    }
}

#[cfg(feature = "metal")]
fn load_wav_mono_16k(path: &Path) -> Result<AudioBuffer, String> {
    let samples = crate::audio::load_wav_mono_16k(path)?;
    Ok(AudioBuffer { samples, sample_rate: 16_000 })
}

#[cfg(feature = "metal")]
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

// ─── Algorithm A: Forced-per-phrase ──────────────────────────────────────────

#[cfg(feature = "metal")]
fn degenerate_stubs(phrase: &PhraseAnchor) -> Vec<AlignedWord> {
    phrase
        .words
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let t = phrase.time_sec + (i as f64) * 0.05;
            AlignedWord {
                word: w.clone(),
                normalized: w.to_lowercase(),
                start: t,
                end: t + 0.05,
                confidence: 0.05,
            }
        })
        .collect()
}

#[cfg(feature = "metal")]
fn run_phrase_alignment(
    aligner: &ForcedAligner,
    vocals: &AudioBuffer,
    phrases: &[PhraseAnchor],
    on_progress: &(dyn Fn(usize, usize) + Send + Sync),
) -> Vec<AlignedWord> {
    const PAD: f64 = 1.5;
    let mut out: Vec<AlignedWord> = Vec::new();

    for i in 0..phrases.len() {
        on_progress(i, phrases.len());

        if phrases[i].words.is_empty() {
            continue;
        }

        let slice: Vec<PhraseAnchor> = if i + 1 < phrases.len() {
            vec![phrases[i].clone(), phrases[i + 1].clone()]
        } else {
            vec![phrases[i].clone()]
        };

        let take_n = phrases[i].words.len();
        match aligner.align_with_phrases(vocals, &slice, PAD) {
            Ok(aligned) => {
                let phrase_words: Vec<AlignedWord> =
                    aligned.into_iter().take(take_n).collect();
                out.extend(phrase_words);
            }
            Err(e) => {
                eprintln!("[aligner/A] phrase {} error: {}", i, e);
                out.extend(degenerate_stubs(&phrases[i]));
            }
        }
    }

    on_progress(phrases.len(), phrases.len());

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

    out
}

#[cfg(feature = "metal")]
async fn try_phrase_forced_alignment(
    dir: &Path,
    lrc_text: &str,
    model: WhisperModel,
    on_progress: &(dyn Fn(usize, usize) + Send + Sync),
) -> Option<Vec<WordEntry>> {
    let vocals_path = dir.join("vocals.mp3");
    if !vocals_path.exists() {
        return None;
    }
    let vocals = match load_wav_mono_16k(&vocals_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[aligner/A] WAV load failed: {}", e);
            return None;
        }
    };

    let lrc_lines = crate::lyrics::parse_lrc(lrc_text);
    if lrc_lines.is_empty() {
        return None;
    }

    let language = detect_lrc_language(lrc_text)
        .unwrap_or("it")
        .to_string();

    let phrases: Vec<PhraseAnchor> = lrc_lines
        .iter()
        .map(|l| PhraseAnchor {
            time_sec: l.ts_ms as f64 / 1000.0,
            words: l
                .text
                .split_whitespace()
                .map(|w| w.to_string())
                .collect(),
        })
        .collect();

    let config = AlignmentConfig {
        model,
        language,
        ..Default::default()
    };
    let aligner = match ForcedAligner::new(config).await {
        Ok(a) => a,
        Err(e) => {
            eprintln!("[aligner/A] model load failed: {}", e);
            return None;
        }
    };

    let aligned = run_phrase_alignment(&aligner, &vocals, &phrases, on_progress);

    // Map AlignedWord → WordEntry. Mapping is 1:1: align_with_phrases returns
    // one AlignedWord per word in phrase.words (lib.rs:402-408), and we
    // pre-truncated the lookahead-phrase output via take_n.
    let mut out: Vec<WordEntry> = Vec::with_capacity(aligned.len());
    let mut cursor = 0usize;
    for (line_idx, phrase) in phrases.iter().enumerate() {
        for _ in &phrase.words {
            if cursor >= aligned.len() {
                break;
            }
            let aw = &aligned[cursor];
            let start_ms = (aw.start * 1000.0).round() as u64;
            let mut end_ms = (aw.end * 1000.0).round() as u64;
            if end_ms <= start_ms {
                end_ms = start_ms + 50;
            }
            out.push(WordEntry {
                word: aw.word.clone(),
                start_ms,
                end_ms,
                line: Some(line_idx),
            });
            cursor += 1;
        }
    }

    if out.is_empty() {
        None
    } else {
        eprintln!("[aligner/A] aligned {} words across {} phrases", out.len(), phrases.len());
        Some(out)
    }
}

// ─── Algorithm B: Free-transcribe-per-phrase ─────────────────────────────────

#[cfg(feature = "metal")]
fn slice_audio(full: &AudioBuffer, start_ms: u64, end_ms: u64) -> AudioBuffer {
    let sr = full.sample_rate as u64;
    let s_idx = ((start_ms * sr) / 1000) as usize;
    let e_idx = (((end_ms * sr) / 1000) as usize).min(full.samples.len());
    let samples = if s_idx < e_idx {
        full.samples[s_idx..e_idx].to_vec()
    } else {
        Vec::new()
    };
    AudioBuffer {
        samples,
        sample_rate: full.sample_rate,
    }
}

/// Normalized Levenshtein distance ∈ [0.0, 1.0]. Two-row DP.
#[cfg(feature = "metal")]
fn levenshtein_normalized(a: &str, b: &str) -> f64 {
    let av: Vec<char> = a.chars().collect();
    let bv: Vec<char> = b.chars().collect();
    if av.is_empty() && bv.is_empty() {
        return 0.0;
    }
    if av.is_empty() || bv.is_empty() {
        return 1.0;
    }
    let n = av.len();
    let m = bv.len();
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr: Vec<usize> = vec![0; m + 1];
    for i in 1..=n {
        curr[0] = i;
        for j in 1..=m {
            let cost = if av[i - 1] == bv[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1)
                .min(curr[j - 1] + 1)
                .min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    let dist = prev[m] as f64;
    let max_len = n.max(m) as f64;
    dist / max_len
}

/// Monotone fuzzy-match LRC tokens to transcribed words via Needleman-Wunsch.
/// LRC tokens unmatched receive interpolated timing between matched neighbors.
/// Returns (mapped WordEntry list, count of real matches).
#[cfg(feature = "metal")]
fn fuzzy_match_lrc_to_transcript(
    lrc_tokens: &[String],
    lrc_text_orig: &str,
    tx_words: &[AlignedWord],
    slice_start_ms: u64,
    line_idx: usize,
) -> (Vec<WordEntry>, usize) {
    let n = lrc_tokens.len();
    let m = tx_words.len();
    if n == 0 {
        return (Vec::new(), 0);
    }

    let lrc_orig_tokens: Vec<&str> = lrc_text_orig.split_whitespace().collect();

    if m == 0 {
        // No transcribed words: emit empty list, real_matches=0 → caller
        // treats as failure and either retries or applies fallback.
        return (Vec::new(), 0);
    }

    // Match cost matrix.
    let mut mc = vec![vec![1.0_f64; m]; n];
    for i in 0..n {
        for j in 0..m {
            let tx_norm = letters_only_lower(&tx_words[j].word);
            mc[i][j] = levenshtein_normalized(&lrc_tokens[i], &tx_norm);
        }
    }

    // Needleman-Wunsch monotone alignment.
    // dp[i][j] = min cost aligning lrc[0..i] with tx[0..j].
    // gap_cost = 1.0 (insert/delete LRC token or transcribed word).
    const GAP: f64 = 1.0;
    const MATCH_THR: f64 = 0.4;
    let mut dp = vec![vec![0.0_f64; m + 1]; n + 1];
    for i in 0..=n {
        dp[i][0] = i as f64 * GAP;
    }
    for j in 0..=m {
        dp[0][j] = j as f64 * GAP;
    }
    for i in 1..=n {
        for j in 1..=m {
            let match_c = if mc[i - 1][j - 1] < MATCH_THR {
                dp[i - 1][j - 1] + mc[i - 1][j - 1]
            } else {
                f64::INFINITY
            };
            let del_lrc = dp[i - 1][j] + GAP;
            let del_tx = dp[i][j - 1] + GAP;
            dp[i][j] = match_c.min(del_lrc).min(del_tx);
        }
    }

    // Trace back: build per-LRC-token assignment to tx index (or None).
    let mut assignment: Vec<Option<usize>> = vec![None; n];
    {
        let mut i = n;
        let mut j = m;
        while i > 0 && j > 0 {
            let here = dp[i][j];
            let match_c = if mc[i - 1][j - 1] < MATCH_THR {
                dp[i - 1][j - 1] + mc[i - 1][j - 1]
            } else {
                f64::INFINITY
            };
            if (here - match_c).abs() < 1e-9 {
                assignment[i - 1] = Some(j - 1);
                i -= 1;
                j -= 1;
            } else if (here - (dp[i - 1][j] + GAP)).abs() < 1e-9 {
                i -= 1;
            } else {
                j -= 1;
            }
        }
    }

    // Build per-LRC-token (start_ms, end_ms, real?) list, interpolating when None.
    let mut times: Vec<Option<(u64, u64)>> = vec![None; n];
    for (i, slot) in assignment.iter().enumerate() {
        if let Some(j) = slot {
            let s_ms = (tx_words[*j].start * 1000.0).round() as i64
                + slice_start_ms as i64;
            let e_ms = (tx_words[*j].end * 1000.0).round() as i64
                + slice_start_ms as i64;
            let s_ms = s_ms.max(0) as u64;
            let mut e_ms = e_ms.max(0) as u64;
            if e_ms <= s_ms {
                e_ms = s_ms + 60;
            }
            times[i] = Some((s_ms, e_ms));
        }
    }

    // Linear interpolate gaps between matched anchors.
    let real_matches = times.iter().filter(|t| t.is_some()).count();
    if real_matches > 0 && real_matches < n {
        let mut last_anchor: Option<(usize, u64)> = None; // (idx, end_ms)
        for i in 0..n {
            if let Some((s, e)) = times[i] {
                if let Some((prev_i, prev_end)) = last_anchor {
                    if i > prev_i + 1 {
                        let gap = (s.saturating_sub(prev_end)) as f64;
                        let steps = (i - prev_i) as f64;
                        for k in 1..(i - prev_i) {
                            let frac = k as f64 / steps;
                            let mid = prev_end as f64 + gap * frac;
                            let ws = mid.round() as u64;
                            let we = ws + 100;
                            times[prev_i + k] = Some((ws, we));
                        }
                    }
                }
                last_anchor = Some((i, e));
            }
        }
        // Fill leading unmatched: extrapolate backward from first anchor.
        if let Some(first_idx) = (0..n).find(|&i| times[i].is_some()) {
            if first_idx > 0 {
                let (fs, _) = times[first_idx].unwrap();
                let step = 200_u64;
                for k in 0..first_idx {
                    let offset = ((first_idx - k) as u64) * step;
                    let ws = fs.saturating_sub(offset);
                    times[k] = Some((ws, ws + 100));
                }
            }
        }
        // Fill trailing unmatched: extrapolate forward from last anchor.
        if let Some(last_idx) = (0..n).rev().find(|&i| times[i].is_some()) {
            if last_idx + 1 < n {
                let (_, le) = times[last_idx].unwrap();
                let step = 200_u64;
                for k in (last_idx + 1)..n {
                    let offset = ((k - last_idx) as u64) * step;
                    let ws = le + offset;
                    times[k] = Some((ws, ws + 100));
                }
            }
        }
    }

    // Build WordEntry list using LRC original tokens (preserves UI text).
    let mut out: Vec<WordEntry> = Vec::with_capacity(n);
    for i in 0..n {
        let Some((start_ms, end_ms)) = times[i] else {
            continue;
        };
        let word = lrc_orig_tokens
            .get(i)
            .map(|s| (*s).to_string())
            .unwrap_or_else(|| lrc_tokens[i].clone());
        let clean = clean_token(&word);
        if clean.is_empty() {
            continue;
        }
        out.push(WordEntry {
            word: clean,
            start_ms,
            end_ms,
            line: Some(line_idx),
        });
    }

    (out, real_matches)
}

#[cfg(feature = "metal")]
async fn try_free_transcribe_alignment(
    dir: &Path,
    lrc_text: &str,
    model: WhisperModel,
    on_progress: &(dyn Fn(usize, usize) + Send + Sync),
) -> Option<Vec<WordEntry>> {
    let vocals_path = dir.join("vocals.mp3");
    if !vocals_path.exists() {
        return None;
    }
    let vocals = match load_wav_mono_16k(&vocals_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[aligner/B] WAV load failed: {}", e);
            return None;
        }
    };

    let lrc_lines = crate::lyrics::parse_lrc(lrc_text);
    if lrc_lines.is_empty() {
        return None;
    }

    let language = detect_lrc_language(lrc_text)
        .unwrap_or("it")
        .to_string();

    let config = AlignmentConfig {
        model,
        language,
        ..Default::default()
    };
    let aligner = match ForcedAligner::new(config).await {
        Ok(a) => a,
        Err(e) => {
            eprintln!("[aligner/B] model load failed: {}", e);
            return None;
        }
    };

    let total_ms: u64 =
        (vocals.samples.len() as u64 * 1000) / vocals.sample_rate as u64;
    const PRE_PAD_MS: i64 = 200;
    const POST_PAD_MS: i64 = 500;

    let mut out: Vec<WordEntry> = Vec::new();

    for (idx, line) in lrc_lines.iter().enumerate() {
        on_progress(idx, lrc_lines.len());

        let line_end_ms = lrc_lines
            .get(idx + 1)
            .map(|l| l.ts_ms)
            .unwrap_or(line.ts_ms + 5000);
        let lrc_tokens: Vec<String> = line
            .text
            .split_whitespace()
            .map(letters_only_lower)
            .filter(|s| !s.is_empty())
            .collect();
        if lrc_tokens.is_empty() {
            continue;
        }

        let s_ms = ((line.ts_ms as i64 - PRE_PAD_MS).max(0) as u64).min(total_ms);
        let e_ms = ((line_end_ms as i64 + POST_PAD_MS) as u64).min(total_ms);
        let mut accepted: Option<Vec<WordEntry>> = None;
        if e_ms > s_ms + 100 {
            let slice = slice_audio(&vocals, s_ms, e_ms);
            if !slice.samples.is_empty() {
                match aligner.transcribe_segment(&slice) {
                    Ok(tx_words) => {
                        let (mapped, _real_matches) = fuzzy_match_lrc_to_transcript(
                            &lrc_tokens,
                            &line.text,
                            &tx_words,
                            s_ms,
                            idx,
                        );
                        accepted = Some(mapped);
                    }
                    Err(e) => {
                        eprintln!("[aligner/B] line {} transcribe error: {}", idx, e);
                    }
                }
            }
        }

        if let Some(words) = accepted.filter(|v| !v.is_empty()) {
            // Clamp to LRC neighbor bounds to enforce monotonicity vs adjacent lines.
            for mut w in words {
                w.start_ms = w.start_ms.clamp(line.ts_ms, line_end_ms.saturating_sub(50));
                if w.end_ms <= w.start_ms {
                    w.end_ms = w.start_ms + 60;
                }
                w.end_ms = w.end_ms.min(line_end_ms);
                out.push(w);
            }
        } else {
            // Final fallback: distribute LRC tokens proportionally over line window.
            let tokens: Vec<&str> = line.text.split_whitespace().collect();
            distribute_words_in_line(&tokens, idx, line.ts_ms, line_end_ms, &mut out);
        }
    }

    on_progress(lrc_lines.len(), lrc_lines.len());

    // Global monotonicity safety net.
    let mut cursor = 0_u64;
    for w in out.iter_mut() {
        if w.start_ms < cursor {
            w.start_ms = cursor;
        }
        if w.end_ms <= w.start_ms {
            w.end_ms = w.start_ms + 60;
        }
        cursor = w.start_ms;
    }

    if out.is_empty() {
        None
    } else {
        eprintln!("[aligner/B] aligned {} words across {} lines", out.len(), lrc_lines.len());
        Some(out)
    }
}

// ─── Public entry point ──────────────────────────────────────────────────────

/// Run word alignment for a song.
///
/// GPU gating:
/// - Without Metal: skip alignment entirely; write empty `[]` so Player.jsx
///   falls back to per-line proportional distribution.
/// - With Metal: dispatch to user-selected algorithm (forced-per-phrase
///   default, or free-transcribe-per-phrase).
pub async fn run_alignment(
    app: &AppHandle,
    dir: &Path,
    lrc: Option<&str>,
    on_phrase_progress: &(dyn Fn(usize, usize) + Send + Sync),
) -> Result<Value, String> {
    let words_path = dir.join("words.json");
    if words_path.exists() {
        let s = std::fs::read_to_string(&words_path).map_err(|e| e.to_string())?;
        return serde_json::from_str(&s).map_err(|e| e.to_string());
    }

    #[cfg(not(feature = "metal"))]
    {
        let _ = (app, lrc, on_phrase_progress);
        eprintln!("[aligner] no GPU feature — skipping word alignment, line-level only");
        return write_empty_words(dir);
    }

    #[cfg(feature = "metal")]
    {
        let lrc_text = match lrc.filter(|t| t.contains('[')) {
            Some(t) => t,
            None => {
                eprintln!("[aligner] no synced LRC — writing empty words.json");
                return write_empty_words(dir);
            }
        };

        let settings = crate::settings::load_settings(app);
        let mode = settings.alignment_mode;
        let model = match settings.whisper_model {
            crate::settings::WhisperModelChoice::Medium => WhisperModel::Medium,
            crate::settings::WhisperModelChoice::LargeV3Turbo => WhisperModel::LargeV3Turbo,
        };

        let words_opt = match mode {
            AlignmentMode::ForcedPerPhrase => {
                try_phrase_forced_alignment(dir, lrc_text, model, on_phrase_progress).await
            }
            AlignmentMode::FreeTranscribePerPhrase => {
                try_free_transcribe_alignment(dir, lrc_text, model, on_phrase_progress).await
            }
        };

        match words_opt {
            Some(words) => write_words_json(dir, &words),
            None => {
                eprintln!("[aligner] alignment failed; writing empty words.json");
                write_empty_words(dir)
            }
        }
    }
}

// ─── Tauri commands ──────────────────────────────────────────────────────────

#[tauri::command]
pub fn get_whisper_model(app: AppHandle) -> String {
    if !cfg!(feature = "metal") {
        return "disabled".into();
    }
    crate::settings::load_settings(&app).whisper_model.as_str().into()
}

#[tauri::command]
pub fn set_whisper_model(app: AppHandle, model: String) -> Result<(), String> {
    let parsed = crate::settings::WhisperModelChoice::from_str(&model)
        .ok_or_else(|| format!("unknown whisper model: {}", model))?;
    let mut settings = crate::settings::load_settings(&app);
    settings.whisper_model = parsed;
    crate::settings::save_settings(&app, &settings)
}

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
