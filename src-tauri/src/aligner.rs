use std::path::{Path, PathBuf};

use aligner_pipeline::{AlignedWord, AudioBuffer};
use aligner_whisper::{AlignmentConfig, ForcedAligner, WhisperModel};
use serde_json::{json, Value};
use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;

use crate::vad::VocalIntervals;

const PER_LINE_SHIFT_MIN_MS: u64 = 1500;

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

#[derive(Debug, Clone)]
struct WhisperSegment {
    text: String,
    start_ms: u64,
    end_ms: u64,
    tokens_lower: Vec<String>,
}

/// Detect language of LRC text by counting stopword hits. Returns ISO-639-1 code
/// or `None` if no strong signal (caller should use `auto`).
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

fn vowel_count(word: &str) -> usize {
    word.chars()
        .filter(|c| matches!(c.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u' | 'y' | 'à' | 'è' | 'é' | 'ì' | 'ò' | 'ù'))
        .count()
        .max(1)
}

fn clean_token(raw: &str) -> String {
    raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '-')
        .to_string()
}

fn letters_only_lower(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Parse whisper-apr segments into WhisperSegment list (drops tag-only segments like [Music]).
fn parse_whisper_segments(result: &Value) -> Result<Vec<WhisperSegment>, String> {
    let segments = result["segments"]
        .as_array()
        .ok_or("missing segments in whisper-apr output")?;

    let mut out: Vec<WhisperSegment> = Vec::new();
    for seg in segments {
        let text = seg["text"].as_str().unwrap_or("").trim().to_string();
        if text.is_empty() {
            continue;
        }
        // Skip bracketed-tag-only segments (e.g. "[Music]", "[Musica]", "(singing...)")
        let stripped: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        if stripped.starts_with('[') && stripped.ends_with(']') {
            continue;
        }
        if stripped.starts_with('(') && stripped.ends_with(')') {
            continue;
        }
        let start_s = seg["start"].as_f64().unwrap_or(0.0);
        let end_s = seg["end"].as_f64().unwrap_or(start_s);
        if end_s <= start_s {
            continue;
        }
        let tokens_lower: Vec<String> = text
            .split_whitespace()
            .map(letters_only_lower)
            .filter(|s| !s.is_empty())
            .collect();
        if tokens_lower.is_empty() {
            continue;
        }
        out.push(WhisperSegment {
            text,
            start_ms: (start_s * 1000.0) as u64,
            end_ms: (end_s * 1000.0) as u64,
            tokens_lower,
        });
    }
    if out.is_empty() {
        return Err("whisper-apr produced no usable segments".into());
    }
    Ok(out)
}

/// Distribute words within a line time window by vowel count weight.
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

/// Whisper-only alignment (no LRC): emit words distributed within each segment.
fn whisper_only_words(segments: &[WhisperSegment]) -> Result<Vec<WordEntry>, String> {
    let mut words: Vec<WordEntry> = Vec::new();
    for seg in segments {
        let tokens: Vec<&str> = seg.text.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }
        let weights: Vec<f64> = tokens.iter().map(|w| vowel_count(w) as f64).collect();
        let total_w: f64 = weights.iter().sum::<f64>().max(1.0);
        let dur_ms = (seg.end_ms - seg.start_ms) as f64;
        let mut t = seg.start_ms as f64;
        for (i, w) in tokens.iter().enumerate() {
            let word_dur = dur_ms * (weights[i] / total_w);
            let ws = t;
            let we = if i == tokens.len() - 1 { seg.end_ms as f64 } else { t + word_dur };
            t = we;
            let clean = clean_token(w);
            if clean.is_empty() {
                continue;
            }
            words.push(WordEntry {
                word: clean,
                start_ms: ws.round() as u64,
                end_ms: we.round() as u64,
                line: None,
            });
        }
    }
    if words.is_empty() {
        return Err("whisper-apr produced no words".into());
    }
    Ok(words)
}

async fn run_whisper(
    app: &AppHandle,
    dir: &Path,
    language: Option<&str>,
) -> Result<Value, String> {
    let vocals_path = dir.join("vocals.wav");
    if !vocals_path.exists() {
        return Err("vocals.wav not found — re-process song to generate it".into());
    }

    let mut args: Vec<String> = vec![
        "transcribe".into(),
        "-f".into(),
        vocals_path.to_string_lossy().into_owned(),
        "--model".into(),
        "small".into(),
        "--split-on-word".into(),
        "--word-timestamps".into(),
        "--no-prints".into(),
        "-o".into(),
        "json".into(),
    ];
    if let Some(lang) = language {
        args.push("-l".into());
        args.push(lang.into());
    }

    eprintln!("[aligner/whisper-apr] invoking sidecar with args: {:?}", args);
    let out = app
        .shell()
        .sidecar("whisper-apr")
        .map_err(|e| format!("whisper-apr sidecar: {}", e))?
        .args(args)
        .output()
        .await
        .map_err(|e| format!("whisper-apr exec: {}", e))?;

    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    eprintln!(
        "[aligner/whisper-apr] exit={:?} stdout_len={} stderr_len={}",
        out.status.code(),
        stdout.len(),
        stderr.len()
    );
    if !stderr.is_empty() {
        eprintln!("[aligner/whisper-apr] stderr:\n{}", &stderr[..stderr.len().min(4000)]);
    }

    if !out.status.success() {
        return Err(format!(
            "whisper-apr failed (exit {:?}).\nstderr: {}\nstdout: {}",
            out.status.code(),
            &stderr[..stderr.len().min(2000)],
            &stdout[..stdout.len().min(2000)]
        ));
    }

    serde_json::from_str(&stdout)
        .map_err(|e| format!("parse whisper-apr JSON: {} (stdout: {})", e, &stdout[..stdout.len().min(200)]))
}

/// Fallback: build words from LRC with per-line VAD onset shift.
/// Used when no whisper alignment is available.
fn build_words_from_lrc_vad(lrc: &str, vad: &VocalIntervals) -> Result<Vec<WordEntry>, String> {
    let lines = crate::lyrics::parse_lrc(lrc);
    if lines.is_empty() {
        return Err("LRC has no timestamped lines".into());
    }

    let mut words: Vec<WordEntry> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let line_start = line.ts_ms;
        let line_end = lines.get(i + 1).map(|l| l.ts_ms).unwrap_or(line_start + 4000);
        if line_end <= line_start {
            continue;
        }

        let effective_start = match vad.first_vocal_in_window(line_start, line_end) {
            Some(onset) if onset > line_start + PER_LINE_SHIFT_MIN_MS => onset,
            _ => line_start,
        };

        let tokens: Vec<&str> = line.text.split_whitespace().collect();
        distribute_words_in_line(&tokens, i, effective_start, line_end, &mut words);
    }
    if words.is_empty() {
        return Err("LRC produced no words".into());
    }
    Ok(words)
}

/// Coverage: fraction of LRC-line tokens present in the whisper segment tokens.
/// Returns (coverage, hits).
fn coverage(lrc_tokens: &[String], seg_tokens: &[String]) -> (f64, usize) {
    if lrc_tokens.is_empty() {
        return (0.0, 0);
    }
    let seg_set: std::collections::HashSet<&String> = seg_tokens.iter().collect();
    let hits = lrc_tokens.iter().filter(|t| seg_set.contains(t)).count();
    (hits as f64 / lrc_tokens.len() as f64, hits)
}

/// Map each LRC line → whisper segment (best Jaccard + temporal prior), allowing
/// multiple lines to share a segment. Then enforce monotonicity: if a line's best
/// segment would regress, drop it (None → fallback timing later).
fn align_lines_to_segments(
    lrc_lines: &[crate::lyrics::LrcLine],
    segments: &[WhisperSegment],
) -> Vec<Option<usize>> {
    let n = lrc_lines.len();
    let m = segments.len();
    if n == 0 || m == 0 {
        return vec![None; n];
    }

    let line_tokens: Vec<Vec<String>> = lrc_lines
        .iter()
        .map(|l| {
            l.text
                .split_whitespace()
                .map(letters_only_lower)
                .filter(|s| !s.is_empty())
                .collect()
        })
        .collect();

    let score = |i: usize, j: usize| -> f64 {
        let (cov, hits) = coverage(&line_tokens[i], &segments[j].tokens_lower);
        // Require at least 2 hits and ≥50% of LRC tokens present.
        if hits < 2 || cov < 0.5 {
            return f64::NEG_INFINITY;
        }
        // Hard gate: LRC line must fall within ±60s of whisper segment mid-point.
        let seg_mid = (segments[j].start_ms + segments[j].end_ms) as f64 / 2.0;
        let dt_s = (lrc_lines[i].ts_ms as f64 - seg_mid).abs() / 1000.0;
        if dt_s > 60.0 {
            return f64::NEG_INFINITY;
        }
        let prior = (-dt_s / 15.0).exp() * 0.3;
        cov + prior
    };

    let mut mapping: Vec<Option<usize>> = (0..n)
        .map(|i| {
            let mut best: Option<(usize, f64)> = None;
            for j in 0..m {
                let s = score(i, j);
                if s.is_finite() && best.map_or(true, |(_, bs)| s > bs) {
                    best = Some((j, s));
                }
            }
            best.map(|(j, _)| j)
        })
        .collect();

    // Enforce monotonic non-decreasing segment indices.
    let mut last: Option<usize> = None;
    for i in 0..n {
        if let Some(j) = mapping[i] {
            if let Some(prev) = last {
                if j < prev {
                    mapping[i] = None;
                    continue;
                }
            }
            last = Some(j);
        }
    }

    mapping
}

/// Build words by mapping each LRC line to a whisper segment (best similarity, monotonic).
/// When multiple contiguous LRC lines share the same whisper segment, the segment time window
/// is split using each inner line's LRC ts_ms as anchor (LRC already has line-level timing).
/// The group's leading edge is bumped by VAD to skip intro silence / "[Musica]" tags whisper
/// sometimes absorbs into the first segment.
/// Unmatched lines fall back to LRC+VAD timing clamped to matched-neighbor boundaries.
fn build_words_from_lrc_and_whisper(
    lrc: &str,
    segments: &[WhisperSegment],
    vad: &VocalIntervals,
) -> Result<Vec<WordEntry>, String> {
    let lrc_lines = crate::lyrics::parse_lrc(lrc);
    if lrc_lines.is_empty() {
        return Err("LRC has no timestamped lines".into());
    }

    let mapping = align_lines_to_segments(&lrc_lines, segments);
    let n = lrc_lines.len();

    // Per-line resolved [start_ms, end_ms].
    let mut line_windows: Vec<Option<(u64, u64)>> = vec![None; n];

    // 1) Resolve matched lines — group consecutive lines sharing the same segment.
    let mut i = 0;
    while i < n {
        let Some(seg_idx) = mapping[i] else {
            i += 1;
            continue;
        };
        let mut k = i + 1;
        while k < n && mapping[k] == Some(seg_idx) {
            k += 1;
        }
        let seg = &segments[seg_idx];
        let group_lines: Vec<usize> = (i..k).collect();

        // Multi-line groups mean whisper merged several lines (often absorbing intro
        // "[Musica]" or silence). Use the first LRC line ts as lower bound for group
        // start so the group doesn't begin before the singer actually starts.
        // Single-line groups trust whisper start directly.
        let first_lrc_ts = lrc_lines[group_lines[0]].ts_ms;
        let raw_start = if group_lines.len() > 1 {
            seg.start_ms.max(first_lrc_ts)
        } else {
            seg.start_ms
        };
        let group_start = raw_start.min(seg.end_ms.saturating_sub(200));

        let mut cursor = group_start;
        for (k_idx, &idx) in group_lines.iter().enumerate() {
            let is_last = k_idx + 1 == group_lines.len();
            let start = if k_idx == 0 {
                group_start
            } else {
                lrc_lines[idx]
                    .ts_ms
                    .clamp(cursor, seg.end_ms.saturating_sub(100))
            };
            let end = if is_last {
                seg.end_ms
            } else {
                let next_ts = lrc_lines[group_lines[k_idx + 1]].ts_ms;
                next_ts.clamp(start + 100, seg.end_ms)
            };
            line_windows[idx] = Some((start, end.max(start + 100)));
            cursor = end;
        }
        i = k;
    }

    // 2) Resolve unmatched lines — clamp LRC window to matched-neighbor anchors,
    //    then apply per-line VAD onset shift.
    for idx in 0..n {
        if line_windows[idx].is_some() {
            continue;
        }
        let line = &lrc_lines[idx];
        let prev_end: Option<u64> = (0..idx)
            .rev()
            .find_map(|k| line_windows[k].map(|(_, e)| e));
        let next_start: Option<u64> = (idx + 1..n)
            .find_map(|k| line_windows[k].map(|(s, _)| s));

        let line_start_orig = line.ts_ms;
        let line_end_orig = lrc_lines
            .get(idx + 1)
            .map(|l| l.ts_ms)
            .unwrap_or(line_start_orig + 4000);

        let clamped_start = match prev_end {
            Some(p) if line_start_orig < p => p,
            _ => line_start_orig,
        };
        let clamped_end = match next_start {
            Some(ns) if line_end_orig > ns => ns,
            _ => line_end_orig,
        };

        if clamped_end <= clamped_start + 100 {
            continue;
        }

        let effective_start = match vad.first_vocal_in_window(clamped_start, clamped_end) {
            Some(onset) if onset > clamped_start + PER_LINE_SHIFT_MIN_MS => onset,
            _ => clamped_start,
        };

        if clamped_end <= effective_start {
            continue;
        }
        line_windows[idx] = Some((effective_start, clamped_end));
    }

    // 3) Distribute words within each resolved window.
    let mut words: Vec<WordEntry> = Vec::new();
    for idx in 0..n {
        let Some((start_ms, end_ms)) = line_windows[idx] else {
            continue;
        };
        let tokens: Vec<&str> = lrc_lines[idx].text.split_whitespace().collect();
        distribute_words_in_line(&tokens, idx, start_ms, end_ms, &mut words);
    }

    if words.is_empty() {
        return Err("alignment produced no words".into());
    }
    Ok(words)
}

// ─── Pure-Rust forced alignment ──────────────────────────────────────────────

fn load_wav_mono_16k(path: &Path) -> Result<AudioBuffer, String> {
    let reader = hound::WavReader::open(path).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    let channels = spec.channels as usize;
    let src_rate = spec.sample_rate;

    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => {
            reader.into_samples::<f32>().filter_map(|s| s.ok()).collect()
        }
        hound::SampleFormat::Int => {
            let bits = spec.bits_per_sample as i32;
            let max = (1i64 << (bits - 1)) as f32;
            reader
                .into_samples::<i32>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / max)
                .collect()
        }
    };

    let mono: Vec<f32> = if channels <= 1 {
        raw
    } else {
        raw.chunks(channels)
            .map(|c| c.iter().sum::<f32>() / channels as f32)
            .collect()
    };

    let samples = if src_rate == 16_000 {
        mono
    } else {
        use rubato::{Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};
        let params = SincInterpolationParameters {
            sinc_len: 128,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 128,
            window: WindowFunction::BlackmanHarris2,
        };
        let ratio = 16_000.0 / src_rate as f64;
        let mut resampler = SincFixedIn::<f32>::new(ratio, 2.0, params, mono.len(), 1)
            .map_err(|e| e.to_string())?;
        let out = resampler.process(&[mono], None).map_err(|e| e.to_string())?;
        out.into_iter().next().ok_or("resample produced no output")?
    };

    Ok(AudioBuffer { samples, sample_rate: 16_000 })
}

/// Map `AlignedWord` list back to LRC lines and build `WordEntry` list.
///
/// LRC timestamps are used as hard bounds: no word may start before its line's
/// LRC timestamp or end after the next line's LRC timestamp.  When the DTW
/// center for a line falls outside the LRC window (chunk-assignment error), all
/// words in that line are redistributed by vowel weight within the LRC window.
fn aligned_to_word_entries(
    aligned: &[AlignedWord],
    lrc_lines: &[crate::lyrics::LrcLine],
) -> Vec<WordEntry> {
    if aligned.is_empty() || lrc_lines.is_empty() {
        return vec![];
    }

    let n_lines = lrc_lines.len();

    // Build word→line mapping and group word indices by line.
    let mut line_for_word: Vec<usize> = Vec::with_capacity(aligned.len());
    for (li, line) in lrc_lines.iter().enumerate() {
        for _ in line.text.split_whitespace() {
            line_for_word.push(li);
        }
    }
    let mut line_word_indices: Vec<Vec<usize>> = vec![vec![]; n_lines];
    for (wi, &li) in line_for_word.iter().enumerate() {
        if wi < aligned.len() {
            line_word_indices[li].push(wi);
        }
    }

    // DTW chunk-assignment tolerance: if the mean DTW start for a line deviates
    // from the LRC line timestamp by more than this, fall back to vowel distribution.
    const TOLERANCE_MS: f64 = 5_000.0;

    let mut result: Vec<WordEntry> = Vec::with_capacity(aligned.len());

    for li in 0..n_lines {
        let indices = &line_word_indices[li];
        if indices.is_empty() {
            continue;
        }

        let lrc_start = lrc_lines[li].ts_ms as f64;
        let lrc_end = lrc_lines
            .get(li + 1)
            .map(|l| l.ts_ms as f64)
            .unwrap_or(lrc_start + 5_000.0);

        let dtw_center = indices.iter()
            .map(|&wi| aligned[wi].start * 1_000.0)
            .sum::<f64>()
            / indices.len() as f64;

        if (dtw_center - lrc_start).abs() <= TOLERANCE_MS {
            // DTW is plausible — use it but clamp every word to the LRC window.
            for &wi in indices {
                let aw = &aligned[wi];
                let raw_start = (aw.start * 1_000.0).round() as u64;
                let raw_end   = (aw.end   * 1_000.0).round() as u64;
                let start_ms = raw_start.clamp(lrc_start as u64, (lrc_end - 20.0).max(lrc_start) as u64);
                let end_ms   = raw_end  .clamp(start_ms + 20, lrc_end as u64);
                result.push(WordEntry {
                    word: aw.word.clone(),
                    start_ms,
                    end_ms,
                    line: Some(li),
                });
            }
        } else {
            // DTW chunk assignment was wrong — redistribute by vowel weight.
            let tokens: Vec<&str> = lrc_lines[li].text.split_whitespace().collect();
            distribute_words_in_line(&tokens, li, lrc_start as u64, lrc_end as u64, &mut result);
        }
    }

    result
}

/// Run the pure-Rust forced aligner. Returns None if vocals don't exist or on error.
async fn try_forced_alignment(
    dir: &Path,
    lrc_text: &str,
) -> Option<Vec<WordEntry>> {
    let vocals_path = dir.join("vocals.wav");
    if !vocals_path.exists() {
        return None;
    }

    let vocals = match load_wav_mono_16k(&vocals_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[aligner/forced] WAV load failed: {}", e);
            return None;
        }
    };

    let lrc_lines = crate::lyrics::parse_lrc(lrc_text);
    let lyrics: String = lrc_lines
        .iter()
        .filter(|l| !l.text.trim().is_empty())
        .map(|l| l.text.trim())
        .collect::<Vec<_>>()
        .join(" ");

    if lyrics.split_whitespace().count() == 0 {
        return None;
    }

    let language = detect_lrc_language(lrc_text)
        .unwrap_or("it")
        .to_string();

    let model = if cfg!(feature = "metal") { WhisperModel::Medium } else { WhisperModel::Small };
    let config = AlignmentConfig { model, language, ..Default::default() };

    let aligner = match ForcedAligner::new(config).await {
        Ok(a) => a,
        Err(e) => {
            eprintln!("[aligner/forced] model load failed: {}", e);
            return None;
        }
    };

    match aligner.align(&vocals, &lyrics) {
        Ok(aligned) => {
            let words = aligned_to_word_entries(&aligned, &lrc_lines);
            if words.is_empty() {
                None
            } else {
                eprintln!("[aligner/forced] aligned {} words", words.len());
                Some(words)
            }
        }
        Err(e) => {
            eprintln!("[aligner/forced] align failed: {}", e);
            None
        }
    }
}

fn write_words_json(dir: &Path, words: &[WordEntry]) -> Result<Value, String> {
    let arr: Vec<Value> = words.iter().map(|w| w.to_json()).collect();
    let path = dir.join("words.json");
    let s = serde_json::to_string_pretty(&arr).map_err(|e| e.to_string())?;
    std::fs::write(&path, s).map_err(|e| e.to_string())?;
    Ok(Value::Array(arr))
}

/// Aligns words for a song.
///
/// Strategy:
/// - Compute (or load cached) VAD on vocals.wav.
/// - If LRC is synced: detect language from LRC → run whisper with `-l <lang>`.
///   DP-align LRC lines to whisper segments, retime lines from whisper, distribute
///   words inside line by vowel count. Unmatched lines fall back to VAD+LRC timing,
///   clamped to neighbor anchors.
/// - Otherwise: whisper-only with vowel-proxy distribution inside each segment.
pub async fn run_alignment(
    app: &AppHandle,
    dir: &Path,
    lrc: Option<&str>,
) -> Result<Value, String> {
    let words_path = dir.join("words.json");
    if words_path.exists() {
        let s = std::fs::read_to_string(&words_path).map_err(|e| e.to_string())?;
        return serde_json::from_str(&s).map_err(|e| e.to_string());
    }

    let vad = crate::vad::load_or_compute(dir)?;

    if let Some(lrc_text) = lrc.filter(|t| t.contains('[')) {
        let lang = detect_lrc_language(lrc_text);
        eprintln!("[aligner] detected LRC language: {:?}", lang);

        // Try the pure-Rust forced aligner first; fall back to whisper-apr sidecar.
        if let Some(words) = try_forced_alignment(dir, lrc_text).await {
            return write_words_json(dir, &words);
        }
        eprintln!("[aligner] forced alignment unavailable, falling back to whisper-apr");

        let words = if vad.total_vocal_ms() > 5000 {
            match run_whisper(app, dir, lang).await {
                Ok(result) => match parse_whisper_segments(&result) {
                    Ok(segs) => match build_words_from_lrc_and_whisper(lrc_text, &segs, &vad) {
                        Ok(w) => w,
                        Err(e) => {
                            eprintln!("[aligner] segment-line merge failed, using LRC+VAD only: {}", e);
                            build_words_from_lrc_vad(lrc_text, &vad)?
                        }
                    },
                    Err(e) => {
                        eprintln!("[aligner] whisper segment parse failed, using LRC+VAD only: {}", e);
                        build_words_from_lrc_vad(lrc_text, &vad)?
                    }
                },
                Err(e) => {
                    eprintln!("[aligner] whisper failed, using LRC+VAD only: {}", e);
                    build_words_from_lrc_vad(lrc_text, &vad)?
                }
            }
        } else {
            build_words_from_lrc_vad(lrc_text, &vad)?
        };
        return write_words_json(dir, &words);
    }

    let result = run_whisper(app, dir, None).await?;
    let segments = parse_whisper_segments(&result)?;
    let words = whisper_only_words(&segments)?;
    write_words_json(dir, &words)
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
